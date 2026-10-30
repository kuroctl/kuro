// Navigation des réglages d'un prefab : sections (générales, avancées) → groupes → sous-pages.
// L'ordre est celui du manifest ; rien n'est codé par jeu.
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { type Field, type Manifest, SECTIONS, type SectionKey } from '../manifest'

export interface FieldEntry {
  key: string
  field: Field
}

export interface GroupEntry {
  key: string
  icon?: string | null
  fields: FieldEntry[]
}

export interface SectionEntry {
  key: SectionKey
  groups: GroupEntry[]
}

export function usePrefabSettings(manifest: MaybeRefOrGetter<Manifest | undefined>) {
  const sections = computed<SectionEntry[]>(() => {
    const settings = toValue(manifest)?.settings ?? {}
    return SECTIONS.map((key) => ({
      key,
      groups: Object.entries(settings[key] ?? {}).map(([group, def]) => ({
        key: group,
        icon: def.icon,
        fields: Object.entries(def.fields).map(([field, definition]) => ({ key: field, field: definition })),
      })),
    })).filter((section) => section.groups.length > 0)
  })

  /** Tous les champs, toutes sections confondues (un seul espace de noms). */
  const fields = computed(() => new Map(sections.value.flatMap((s) => s.groups.flatMap((g) => g.fields)).map((f) => [f.key, f.field])))

  function group(section: string, key: string): GroupEntry | undefined {
    return sections.value.find((s) => s.key === section)?.groups.find((g) => g.key === key)
  }

  /** Vrai si l'un des champs modifiés impose un redémarrage. */
  function needsRestart(changed: string[]): boolean {
    return changed.some((key) => fields.value.get(key)?.restart)
  }

  return { sections, fields, group, needsRestart }
}
