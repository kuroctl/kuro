// Valeurs saisies pour un contenu (ADR-0030) : texte du formulaire (une ligne par valeur pour une
// liste) ↔ valeurs envoyées à l'API.
import type { ContentSlot } from './content.api'

type Fields = ContentSlot['fields']

/** Texte saisi → valeurs de l'API ; l'API retire les valeurs vides. */
export function valuesPayload(fields: Fields, texts: Record<string, string>): Record<string, string | string[]> {
  return Object.fromEntries(
    fields.map((field) => {
      const text = texts[field.key] ?? ''
      return [field.key, field.type === 'list' ? text.split('\n') : text]
    }),
  )
}

/** Valeurs d'un contenu → texte du formulaire. */
export function valuesTexts(fields: Fields, values: unknown): Record<string, string> {
  const stored = (values && typeof values === 'object' ? values : {}) as Record<string, unknown>
  return Object.fromEntries(
    fields.map((field) => {
      const value = stored[field.key]
      return [field.key, Array.isArray(value) ? value.join('\n') : typeof value === 'string' ? value : '']
    }),
  )
}

/** Valeurs d'un contenu, à lister sous son nom. */
export function valuesSummary(values: unknown): [string, string][] {
  const stored = (values && typeof values === 'object' ? values : {}) as Record<string, unknown>
  return Object.entries(stored)
    .map(([key, value]): [string, string] => [key, Array.isArray(value) ? value.join(', ') : String(value)])
    .filter(([, text]) => text.length > 0)
}
