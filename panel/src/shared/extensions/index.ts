// Extensions du panel admin (ADR-0051 § 8) : pages, navigation et traductions d'une extension,
// compilées avec le panel. Le build les reçoit par `KURO_PANEL_EXTENSIONS` (`vite.config.ts`) ;
// sans elles, le panel est identique.
//
// Une extension fournit `extension.ts` (`export default definePanelExtension({ … })`) et un
// dossier `pages/`, monté sous `/<nom>/…`. Ses textes sont sous `ext.<nom>.*`.
import type { BuiltinSectionKey, Section, SubnavItem } from '@/shared/layout/sections'

export interface PanelExtensionManifest {
  /** Sections propres, ajoutées au rail avant « Paramètres ». */
  sections?: Section[]
  /** Entrées ajoutées aux sous-menus des sections de Kuro (`users` : « Clients »…). */
  items?: Partial<Record<BuiltinSectionKey, SubnavItem[]>>
  /** Traductions, montées sous `ext.<nom>`. */
  messages?: { fr?: Record<string, unknown>; en?: Record<string, unknown> }
}

/** Extension telle que le build la fournit : son nom (préfixe des routes et des textes). */
export interface PanelExtension extends PanelExtensionManifest {
  name: string
}

export function definePanelExtension(manifest: PanelExtensionManifest): PanelExtensionManifest {
  return manifest
}

/** Navigation de Kuro complétée par celle des extensions, dans l'ordre reçu. */
export function withExtensions(builtin: Section[], extensions: PanelExtension[]): Section[] {
  const merged = builtin.map((section) => ({
    ...section,
    items: [
      ...section.items,
      ...extensions.flatMap((extension) => extension.items?.[section.key as BuiltinSectionKey] ?? []),
    ],
  }))
  const added = extensions.flatMap((extension) => extension.sections ?? [])
  const before = merged.findIndex((section) => section.key === 'settings')
  const at = before < 0 ? merged.length : before
  return [...merged.slice(0, at), ...added, ...merged.slice(at)]
}

/** Traductions des extensions, sous `ext.<nom>`, pour une langue. */
export function extensionMessages(extensions: PanelExtension[], locale: 'fr' | 'en'): Record<string, unknown> {
  return Object.fromEntries(extensions.map((extension) => [extension.name, extension.messages?.[locale] ?? {}]))
}
