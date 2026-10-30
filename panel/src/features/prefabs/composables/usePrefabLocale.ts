// Traductions d'un prefab (ADR-0011), chargées à la demande pour la langue courante, avec repli
// sur l'anglais puis sur la clé.
//
// Ce sont du contenu, pas des gabarits vue-i18n : elles sont lues telles quelles (un `@` ou une
// `{` dans un texte de prefab n'est jamais interprété) et affichées échappées.
import { useQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePrefabsApi, type PrefabMessages } from '../prefabs.api'

const FALLBACK = 'en'

interface PrefabRef {
  id: string
  version: string
}

function lookup(messages: PrefabMessages | undefined, path: string): string | undefined {
  const value = path
    .split('.')
    .reduce<unknown>((node, key) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[key] : undefined), messages)
  return typeof value === 'string' ? value : undefined
}

/** `max_players` → `Max players` : dernier recours quand aucune traduction n'existe. */
export function humanize(key: string): string {
  const text = key.replace(/[_-]+/g, ' ').trim()
  return text.charAt(0).toUpperCase() + text.slice(1)
}

export function usePrefabLocale(prefab: MaybeRefOrGetter<PrefabRef | undefined>) {
  const { locale } = useI18n()
  const api = usePrefabsApi()

  const load = (lang: () => string) =>
    useQuery(
      computed(() => {
        const ref = toValue(prefab)
        return {
          queryKey: ['prefabs', ref?.id, ref?.version, 'locale', lang()],
          // Langue absente du prefab : pas d'erreur, le repli prend le relais.
          queryFn: () => (ref ? api.locale(ref.id, ref.version, lang()).catch(() => ({})) : {}),
          enabled: Boolean(ref),
          staleTime: Infinity,
        }
      }),
    )

  const current = load(() => locale.value)
  const fallback = load(() => FALLBACK)

  return prefabLocale(
    (path) => lookup(current.data.value, path) ?? lookup(fallback.data.value, path),
    () => toValue(prefab)?.id ?? '',
  )
}

/** Libellés d'un prefab à partir d'une fonction de recherche dans ses traductions. */
function prefabLocale(text: (path: string) => string | undefined, id: () => string) {
  return {
    text,
    prefabName: () => text('prefab.name') ?? id(),
    groupTitle: (group: string) => text(`groups.${group}.title`) ?? humanize(group),
    fieldLabel: (field: string) => text(`fields.${field}.label`) ?? humanize(field),
    fieldHelp: (field: string) => text(`fields.${field}.help`),
    optionLabel: (field: string, option: unknown) => text(`fields.${field}.options.${String(option)}`) ?? String(option),
    fileDescription: (file: string) => text(`files.${file}.description`),
    contentTitle: (slot: string) => text(`content.${slot}.title`) ?? humanize(slot),
    contentFieldLabel: (slot: string, field: string) => text(`content.${slot}.fields.${field}.label`) ?? humanize(field),
    contentFieldHelp: (slot: string, field: string) => text(`content.${slot}.fields.${field}.help`),
    actionLabel: (action: string) => text(`actions.${action}.label`) ?? humanize(action),
    actionHelp: (action: string) => text(`actions.${action}.help`),
    actionParamLabel: (action: string, param: string) => text(`actions.${action}.params.${param}.label`) ?? humanize(param),
  }
}

/** Traductions déjà en mémoire (brouillon du Studio) : langue → messages, même repli. */
export function useMessagesLocale(messages: MaybeRefOrGetter<Record<string, PrefabMessages>>, id: MaybeRefOrGetter<string>) {
  const { locale } = useI18n()
  return prefabLocale(
    (path) => lookup(toValue(messages)[locale.value], path) ?? lookup(toValue(messages)[FALLBACK], path),
    () => toValue(id),
  )
}

export type PrefabLocale = ReturnType<typeof usePrefabLocale>
