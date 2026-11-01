// Extensions du panel admin (ADR-0051 § 8), lues au build depuis `KURO_PANEL_EXTENSIONS` :
// `<nom>=<dossier>` séparés par des virgules (dossier relatif au panel, ou absolu). Chaque dossier
// contient `extension.ts` (manifeste) et `pages/` (routes sous `/<nom>/…`).
import { existsSync } from 'node:fs'
import { join, resolve } from 'node:path'

import type { Plugin } from 'vite'

export interface ExtensionDir {
  name: string
  dir: string
}

const NAME = /^[a-z][a-z0-9_]{0,31}$/
const VIRTUAL = 'virtual:kuro-panel-extensions'
const RESOLVED = `\0${VIRTUAL}`

/** Extensions déclarées ; une entrée mal formée arrête le build plutôt que d'être ignorée. */
export function parseExtensions(value: string | undefined, root: string): ExtensionDir[] {
  const entries = (value ?? '')
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
  const seen = new Set<string>()
  return entries.map((entry) => {
    const separator = entry.indexOf('=')
    const name = separator > 0 ? entry.slice(0, separator) : ''
    const dir = resolve(root, entry.slice(separator + 1))
    if (!NAME.test(name)) throw new Error(`KURO_PANEL_EXTENSIONS: invalid entry "${entry}", expected <name>=<dir>`)
    if (seen.has(name)) throw new Error(`KURO_PANEL_EXTENSIONS: extension "${name}" listed twice`)
    if (!existsSync(join(dir, 'extension.ts'))) throw new Error(`KURO_PANEL_EXTENSIONS: ${dir} has no extension.ts`)
    seen.add(name)
    return { name, dir }
  })
}

/** Dossiers de pages pour le routage par fichiers : celui de Kuro, puis `/<nom>/` par extension. */
export function routesFolders(extensions: ExtensionDir[]) {
  return [
    'src/pages',
    ...extensions
      .filter((extension) => existsSync(join(extension.dir, 'pages')))
      .map((extension) => ({ src: join(extension.dir, 'pages'), path: `${extension.name}/` })),
  ]
}

/**
 * Module virtuel `virtual:kuro-panel-extensions` (les manifestes, avec leur nom), et dossiers des
 * extensions déclarés à Tailwind (`@source`) : hors du panel, leurs classes ne seraient pas
 * générées sinon.
 */
export function kuroPanelExtensions(extensions: ExtensionDir[]): Plugin {
  return {
    name: 'kuro-panel-extensions',
    // Avant Tailwind, pour qu'il lise les `@source` ajoutés.
    enforce: 'pre',
    resolveId(id) {
      return id === VIRTUAL ? RESOLVED : undefined
    },
    transform(code, id) {
      if (extensions.length === 0 || !id.replaceAll('\\', '/').endsWith('/src/assets/main.css')) return undefined
      const sources = extensions.map((extension) => `@source ${JSON.stringify(extension.dir.replaceAll('\\', '/'))};`)
      return `${code}\n${sources.join('\n')}\n`
    },
    load(id) {
      if (id !== RESOLVED) return undefined
      const imports = extensions.map(
        (extension, index) => `import m${index} from ${JSON.stringify(join(extension.dir, 'extension.ts').replaceAll('\\', '/'))}`,
      )
      const list = extensions.map((extension, index) => `{ ...m${index}, name: ${JSON.stringify(extension.name)} }`)
      return `${imports.join('\n')}\nexport default [${list.join(', ')}]\n`
    },
  }
}
