// Formulaire d'un groupe de réglages : schéma zod généré à partir des champs du prefab, mêmes
// règles que l'API (types, bornes, motif, options, caractères de contrôle). L'API reste seule
// juge : ses erreurs (`settings.<clé>`) sont replacées sous leur champ.
import { toTypedSchema } from '@vee-validate/zod'
import { useForm } from 'vee-validate'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { z, type ZodTypeAny } from 'zod'

import type { Field } from '../manifest'
import type { FieldEntry } from './usePrefabSettings'

export type FormMode = 'create' | 'edit'

// Saut de ligne ou caractère de contrôle : il pourrait injecter une ligne dans un fichier.
const CONTROL = /[\u0000-\u001f\u007f]/
const CONTROL_EXCEPT_NEWLINE = /[\u0000-\u0008\u000b-\u001f\u007f]/

export function usePrefabForm(fields: FieldEntry[], initial: Record<string, unknown>, mode: FormMode) {
  const { t } = useI18n()

  /** Champ non modifiable ici : fixé au déploiement. */
  const locked = (field: Field) => mode === 'edit' && field.editable === 'deploy'
  /** Secret : jamais renvoyé par l'API ; vide = inchangé (édition) ou généré (création). */
  const isSecret = (field: Field) => field.type === 'secret'

  function schemaOf(field: Field): ZodTypeAny {
    switch (field.type) {
      case 'bool':
        return z.boolean()
      case 'int':
      case 'float':
      case 'slider': {
        let number = z.number({ invalid_type_error: t('prefabs.validation.number') })
        if (field.type === 'int') number = number.int(t('prefabs.validation.integer'))
        if (field.min != null) number = number.min(field.min, t('prefabs.validation.min', { min: field.min }))
        if (field.max != null) number = number.max(field.max, t('prefabs.validation.max', { max: field.max }))
        return field.required ? number : number.optional()
      }
      case 'enum': {
        const options = field.options ?? []
        return z.any().refine((value) => options.includes(value), t('prefabs.validation.options'))
      }
      default: {
        const multiline = field.type === 'text'
        let text: ZodTypeAny = z
          .string()
          .refine((value) => !(multiline ? CONTROL_EXCEPT_NEWLINE : CONTROL).test(value), t('prefabs.validation.control'))
        if (field.max_length) {
          text = text.refine((value) => [...value].length <= field.max_length!, t('prefabs.validation.max_length', { max: field.max_length }))
        }
        if (field.pattern) {
          const pattern = new RegExp(field.pattern)
          text = text.refine((value) => value === '' || pattern.test(value), t('prefabs.validation.pattern'))
        }
        if (field.required && !isSecret(field)) {
          text = text.refine((value) => value.trim() !== '', t('prefabs.validation.required'))
        }
        return text
      }
    }
  }

  const schema = computed(() =>
    toTypedSchema(z.object(Object.fromEntries(fields.map(({ key, field }) => [key, locked(field) ? z.any() : schemaOf(field)])))),
  )

  const initialValues = Object.fromEntries(
    fields.map(({ key, field }) => {
      const value = initial[key] ?? field.default
      if (isSecret(field)) return [key, '']
      if (value === undefined || value === null) {
        return [key, field.type === 'bool' ? false : ['int', 'float', 'slider'].includes(field.type) ? undefined : '']
      }
      return [key, value]
    }),
  )

  const form = useForm({ validationSchema: schema, initialValues })

  /** Valeurs à envoyer : seules celles qui ont changé (un secret vide est ignoré). */
  function changes(values: Record<string, unknown>): Record<string, unknown> {
    return Object.fromEntries(
      fields
        .filter(({ key, field }) => !locked(field) && !(isSecret(field) && values[key] === ''))
        .filter(({ key }) => mode === 'create' || values[key] !== initialValues[key])
        .map(({ key }) => [key, values[key] === '' ? null : values[key]]),
    )
  }

  /** Erreurs de l'API (`settings.<clé>`) replacées sous leur champ. */
  function applyApiErrors(errors: Record<string, string>) {
    for (const [name, message] of Object.entries(errors)) {
      form.setFieldError(name.replace(/^settings\./, ''), message)
    }
  }

  return { form, changes, applyApiErrors, locked, apiFields: fields.map(({ key }) => `settings.${key}`) }
}
