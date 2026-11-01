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

/** Chemin à barres obliques, pour comparer des chemins Windows et POSIX. */
const normalize = (path: string) => path.replaceAll('\\', '/')

/** Import d'un paquet (`vue`, `@scope/pkg/sub`), ni relatif, ni absolu, ni alias, ni virtuel. */
export function isBare(id: string): boolean {
  return !/^(\.|\/|[a-zA-Z]:|\0|@\/|virtual:)/.test(id)
}

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
export function kuroPanelExtensions(extensions: ExtensionDir[], root: string): Plugin {
  const dirs = extensions.map((extension) => normalize(extension.dir))
  const inExtension = (file: string | undefined) => !!file && dirs.some((dir) => normalize(file).startsWith(`${dir}/`))
  return {
    name: 'kuro-panel-extensions',
    // Avant Tailwind (à placer avant lui dans `plugins`), pour qu'il lise les `@source` ajoutés.
    enforce: 'pre',
    async resolveId(id, importer, options) {
      if (id === VIRTUAL) return RESOLVED
      // Import nu (`vue`, `@lucide/vue`…) d'un fichier d'extension : l'extension est hors du
      // panel, sans `node_modules` ; il se résout depuis le panel, avec les mêmes versions.
      if (!isBare(id) || !inExtension(importer)) return undefined
      return this.resolve(id, join(root, 'src', 'main.ts'), { ...options, skipSelf: true })
    },
    // Priorité `pre` au niveau du crochet : Tailwind transforme le CSS avec la même priorité ;
    // placé avant lui dans `plugins`, ce crochet passe le premier.
    transform: {
      order: 'pre',
      handler(code, id) {
        if (extensions.length === 0 || !normalize(id).split('?')[0]!.endsWith('/src/assets/main.css')) return undefined
        const sources = extensions.map((extension) => `@source ${JSON.stringify(normalize(extension.dir))};`)
        // Juste après l'import de Tailwind, avant toute règle.
        return code.replace(/(@import\s+['"]tailwindcss['"];?)/, `$1\n${sources.join('\n')}`)
      },
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
