// Aides de l'éditeur du Studio : problèmes de validation placés dans le texte, et
// autocomplétion du `prefab.yaml` depuis le JSON Schema publié (clés et valeurs permises).
import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete'
import type { Diagnostic } from '@codemirror/lint'
import { isMap, isScalar, isSeq, parseDocument } from 'yaml'

import type { DraftCheck } from './studio.api'

type Schema = Record<string, unknown>

/** `install.steps[0].download` → `['install', 'steps', 0, 'download']`. */
export function segments(path: string): (string | number)[] {
  return path
    .split(/\.|(?=\[)/)
    .filter(Boolean)
    .map((part) => (/^\[\d+\]$/.test(part) ? Number(part.slice(1, -1)) : part))
}

/** Zone du texte qui correspond à un chemin du manifest (sa clé), ou au plus proche parent présent. */
export function locate(text: string, path: string): { from: number; to: number } | undefined {
  const document = parseDocument(text)
  const keys = segments(path)
  for (let length = keys.length; length > 0; length--) {
    const parent = length > 1 ? document.getIn(keys.slice(0, length - 1), true) : document.contents
    const last = keys[length - 1]
    if (isMap(parent)) {
      const pair = parent.items.find((item) => isScalar(item.key) && item.key.value === last)
      const range = pair && isScalar(pair.key) ? pair.key.range : undefined
      if (range) return { from: range[0], to: range[1] }
    } else if (isSeq(parent) && typeof last === 'number') {
      const range = (parent.items[last] as { range?: [number, number, number] } | undefined)?.range
      if (range) return { from: range[0], to: range[1] }
    }
  }
  return undefined
}

/** Position d'un « line N column M » (1-based) d'un message d'erreur, dans le texte. */
function lineColumn(text: string, message: string): number | undefined {
  const match = /line (\d+),? column (\d+)/.exec(message)
  if (!match) return undefined
  const lines = text.split('\n')
  const line = Number(match[1]) - 1
  if (line < 0 || line >= lines.length) return undefined
  return lines.slice(0, line).reduce((total, current) => total + current.length + 1, 0) + Math.max(0, Number(match[2]) - 1)
}

/** Problèmes de la validation qui concernent ce fichier, placés dans son texte. */
export function diagnosticsFor(file: string, text: string, check: DraftCheck | undefined): Diagnostic[] {
  if (!check) return []
  const diagnostics: Diagnostic[] = []
  if (check.reason && check.reason.includes(file === 'prefab.yaml' ? 'prefab.yaml' : file)) {
    const at = lineColumn(text, check.reason) ?? 0
    const message = check.reason.split('\n')[0] ?? check.reason
    diagnostics.push({ from: at, to: Math.min(at + 1, text.length), severity: 'error', message })
  }
  if (file === 'prefab.yaml') {
    for (const issue of check.issues) {
      const range = locate(text, issue.path) ?? { from: 0, to: 0 }
      diagnostics.push({ ...range, severity: 'error', message: `${issue.path} : ${issue.message}` })
    }
  }
  return diagnostics
}

// ---------------------------------------------------------------------------------------------
// Autocomplétion depuis le JSON Schema

function resolve(root: Schema, node: unknown): Schema | undefined {
  let current = node as Schema | undefined
  for (let depth = 0; current && typeof current.$ref === 'string' && depth < 16; depth++) {
    const pointer = current.$ref.replace(/^#\//, '').split('/')
    current = pointer.reduce<unknown>((value, key) => (value as Schema | undefined)?.[key], root) as Schema | undefined
  }
  return current
}

/** Variantes d'un nœud (`oneOf`, `anyOf`, `allOf`), lui compris. */
function variants(root: Schema, node: Schema | undefined): Schema[] {
  const resolved = resolve(root, node)
  if (!resolved) return []
  const nested = ['oneOf', 'anyOf', 'allOf'].flatMap((keyword) =>
    Array.isArray(resolved[keyword]) ? (resolved[keyword] as Schema[]).flatMap((variant) => variants(root, variant)) : [],
  )
  return [resolved, ...nested]
}

function child(root: Schema, node: Schema | undefined, key: string | null): Schema | undefined {
  for (const variant of variants(root, node)) {
    if (key === null) {
      if (variant.items) return resolve(root, variant.items)
      continue
    }
    const properties = variant.properties as Record<string, Schema> | undefined
    if (properties?.[key]) return resolve(root, properties[key])
    const patterns = variant.patternProperties as Record<string, Schema> | undefined
    for (const [pattern, schema] of Object.entries(patterns ?? {})) {
      if (new RegExp(pattern).test(key)) return resolve(root, schema)
    }
    if (variant.additionalProperties && typeof variant.additionalProperties === 'object') {
      return resolve(root, variant.additionalProperties)
    }
  }
  return undefined
}

/** Chemin des clés ouvertes au-dessus de la ligne (de la racine vers l'intérieur), `null` pour un élément de liste. */
export function parentPath(lines: string[], column: number): (string | null)[] {
  const path: (string | null)[] = []
  let want = column
  for (let index = lines.length - 1; index >= 0 && want > 0; index--) {
    const line = lines[index] ?? ''
    if (!line.trim() || line.trim().startsWith('#')) continue
    const indent = line.length - line.trimStart().length
    const dash = /^(\s*)- (.*)$/.exec(line)
    if (dash) {
      const keyColumn = indent + 2
      const inline = /^([\w.-]+):\s*$/.exec(dash[2] ?? '')
      if (inline && keyColumn < want) path.push(inline[1] ?? '')
      if (indent < want) {
        path.push(null)
        // La clé de la liste peut être au même retrait que ses tirets.
        want = indent + 1
      }
      continue
    }
    const key = /^\s*([\w.-]+):\s*(#.*)?$/.exec(line)
    if (key && indent < want) {
      path.push(key[1] ?? '')
      want = indent
    }
  }
  return path.reverse()
}

function describe(schema: Schema | undefined): Pick<Completion, 'detail' | 'info'> {
  const type = schema?.type
  return {
    detail: Array.isArray(type) ? type.join(' | ') : typeof type === 'string' ? type : undefined,
    info: typeof schema?.description === 'string' ? schema.description : undefined,
  }
}

/** Source d'autocomplétion pour le `prefab.yaml`. */
export function schemaCompletion(root: Schema) {
  return (context: CompletionContext): CompletionResult | null => {
    const line = context.state.doc.lineAt(context.pos)
    const before = line.text.slice(0, context.pos - line.from)
    const previous = context.state.doc.sliceString(0, line.from).split('\n').slice(0, -1)

    // Valeur : `clé: …`
    const value = /^(\s*)(- )?([\w.-]+):\s+([\w.-]*)$/.exec(before)
    if (value) {
      const column = (value[1]?.length ?? 0) + (value[2] ? 2 : 0)
      const parents = parentPath(previous, column)
      if (value[2]) parents.push(null)
      const parent = parents.reduce<Schema | undefined>((node, key) => child(root, node, key), root)
      const target = child(root, parent, value[3] ?? '')
      const options = variants(root, target).flatMap((variant) => [
        ...(Array.isArray(variant.enum) ? variant.enum : []),
        ...('const' in variant ? [variant.const] : []),
        ...(variant.type === 'boolean' ? [true, false] : []),
      ])
      if (!options.length) return null
      return {
        from: context.pos - (value[4]?.length ?? 0),
        options: [...new Set(options.map(String))].map((option) => ({ label: option, type: 'enum' })),
      }
    }

    // Clé : début de ligne, éventuellement après `- `.
    const key = /^(\s*)(- )?([\w.-]*)$/.exec(before)
    if (!key || (!context.explicit && !key[3])) return null
    const column = (key[1]?.length ?? 0) + (key[2] ? 2 : 0)
    const parents = parentPath(previous, column)
    if (key[2]) parents.push(null)
    const parent = parents.reduce<Schema | undefined>((node, segment) => child(root, node, segment), root)
    const options = variants(root, parent).flatMap((variant) =>
      Object.entries((variant.properties as Record<string, Schema> | undefined) ?? {}).map(([name, schema]) => ({
        label: name,
        type: 'property',
        apply: `${name}: `,
        ...describe(resolve(root, schema)),
      })),
    )
    if (!options.length) return null
    const unique = [...new Map(options.map((option) => [option.label, option])).values()]
    return { from: context.pos - (key[3]?.length ?? 0), options: unique, validFor: /^[\w.-]*$/ }
  }
}
