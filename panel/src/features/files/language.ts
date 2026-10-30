// Coloration de l'éditeur : selon le `format` déclaré par le prefab, sinon selon l'extension.
import { json } from '@codemirror/lang-json'
import { xml } from '@codemirror/lang-xml'
import { yaml } from '@codemirror/lang-yaml'
import { HighlightStyle, StreamLanguage } from '@codemirror/language'
import { lua } from '@codemirror/legacy-modes/mode/lua'
import { properties } from '@codemirror/legacy-modes/mode/properties'
import { shell } from '@codemirror/legacy-modes/mode/shell'
import { toml } from '@codemirror/legacy-modes/mode/toml'
import type { Extension } from '@codemirror/state'
import { tags } from '@lezer/highlight'

const BY_FORMAT: Record<string, () => Extension> = {
  json: () => json(),
  yaml: () => yaml(),
  xml: () => xml(),
  // `ini` et `cfg` ont la même forme `clé = valeur` que `properties`.
  properties: () => StreamLanguage.define(properties),
  ini: () => StreamLanguage.define(properties),
  cfg: () => StreamLanguage.define(properties),
  toml: () => StreamLanguage.define(toml),
  lua: () => StreamLanguage.define(lua),
  sh: () => StreamLanguage.define(shell),
}

const BY_EXTENSION: Record<string, string> = {
  json: 'json',
  json5: 'json',
  mcmeta: 'json',
  yml: 'yaml',
  yaml: 'yaml',
  xml: 'xml',
  properties: 'properties',
  ini: 'ini',
  cfg: 'cfg',
  conf: 'cfg',
  toml: 'toml',
  lua: 'lua',
  sh: 'sh',
}

/** Format effectif d'un fichier, ou `undefined` pour du texte brut. */
export function formatOf(path: string, declared?: string): string | undefined {
  if (declared && declared in BY_FORMAT) return declared
  const extension = path.split('.').at(-1)?.toLowerCase() ?? ''
  return BY_EXTENSION[extension]
}

export function languageOf(format: string | undefined): Extension {
  return format ? (BY_FORMAT[format]?.() ?? []) : []
}

/** Couleurs du thème (`--syntax-*`), en clair comme en sombre. */
export const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.bool, tags.null, tags.atom], color: 'var(--syntax-keyword)' },
  { tag: [tags.string, tags.special(tags.string)], color: 'var(--syntax-string)' },
  { tag: [tags.number, tags.integer, tags.float], color: 'var(--syntax-number)' },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: 'var(--syntax-comment)', fontStyle: 'italic' },
  {
    tag: [tags.propertyName, tags.definition(tags.propertyName), tags.attributeName, tags.tagName, tags.heading],
    color: 'var(--syntax-property)',
  },
])
