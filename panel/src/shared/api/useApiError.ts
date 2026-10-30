// Traduction des erreurs de l'API (RFC 9457 + `code`, `params`, `fields`).
//
// Usage, au point d'appel :
//   try { await login(form) } catch (error) { fieldErrors.value = handleError(error, ['email']) }
//
// - les erreurs des champs listés sont **retournées** (affichées sous chaque champ) ;
// - tout le reste part en toast traduit, avec la référence de la requête pour le support.
import { isAxiosError } from 'axios'
import { toast } from 'vue-sonner'
import { useI18n } from 'vue-i18n'

import type { ErrorCode, Problem } from './types'

/** Message traduit par nom de champ. */
export type FieldErrors = Record<string, string>

const REQUEST_ID_HEADER = 'x-request-id'

function asProblem(data: unknown): Problem | null {
  return typeof data === 'object' && data !== null && 'code' in data ? (data as Problem) : null
}

export function useApiError() {
  const { t, te } = useI18n()

  function translate(code: ErrorCode, params?: object): string {
    const key = `errors.${code}`
    return te(key) ? t(key, (params ?? {}) as Record<string, unknown>) : t('errors.unknown')
  }

  function handleError(error: unknown, formFields: readonly string[] = []): FieldErrors {
    if (!isAxiosError(error)) {
      toast.error(t('errors.unknown'))
      return {}
    }
    if (!error.response) {
      toast.error(t('errors.network'))
      return {}
    }

    const problem = asProblem(error.response.data)
    const requestId = String(error.response.headers[REQUEST_ID_HEADER] ?? '')
    const description = requestId ? t('errors.reference', { id: requestId.slice(0, 8) }) : undefined
    if (!problem) {
      toast.error(t('errors.unknown'), { description })
      return {}
    }

    const fieldErrors: FieldErrors = {}
    for (const field of problem.fields ?? []) {
      if (formFields.includes(field.field)) {
        fieldErrors[field.field] = translate(field.code, field.params)
      }
    }
    if (Object.keys(fieldErrors).length === 0) {
      toast.error(translate(problem.code, problem.params), { description })
    }
    return fieldErrors
  }

  /** Message traduit d'une erreur, sans toast : pour l'afficher sur place. */
  function message(error: unknown): string {
    const problem = isAxiosError(error) ? asProblem(error.response?.data) : null
    if (problem) return translate(problem.code, problem.params)
    return t(isAxiosError(error) && !error.response ? 'errors.network' : 'errors.unknown')
  }

  return { handleError, message }
}

/** Code d'erreur de l'API (`file_conflict`…), pour réagir à un cas précis. */
export function problemCode(error: unknown): string | undefined {
  return isAxiosError(error) ? asProblem(error.response?.data)?.code : undefined
}
