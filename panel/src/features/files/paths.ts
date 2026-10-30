// Chemins des fichiers d'un serveur : `/<service>/<volume>/…` (ADR-0015).
import type { Manifest } from '@/features/prefabs/manifest'

export function segments(path: string): string[] {
  return path.split('/').filter(Boolean)
}

export function join(directory: string, name: string): string {
  return `/${[...segments(directory), name].join('/')}`
}

export function parent(path: string): string {
  return `/${segments(path).slice(0, -1).join('/')}`
}

export function basename(path: string): string {
  return segments(path).at(-1) ?? ''
}

/** Sous un volume : on peut y créer, renommer, supprimer (les deux premiers niveaux sont fixes). */
export function isEditable(directory: string): boolean {
  return segments(directory).length >= 2
}

/** Même règle que l'API et le daemon pour un nouveau nom. */
export function isValidName(name: string): boolean {
  return (
    name.length > 0 &&
    name !== '.' &&
    name !== '..' &&
    new TextEncoder().encode(name).length <= 255 &&
    !/[/\\]/.test(name) &&
    !/[\u0000-\u001f\u007f-\u009f]/.test(name)
  )
}

export interface DeclaredFile {
  key: string
  path: string
  format: string
  /** Écrit par Kuro à chaque démarrage (fichier d'inscription des paquets, ADR-0030). */
  generated?: boolean
}

/** Chemin du conteneur → chemin des volumes (`/<service>/<volume>/…`), s'il est sous un volume. */
function volumePath(manifest: Manifest, service: string, path: string): string | undefined {
  const volumes = manifest.services[service]?.volumes ?? {}
  for (const [volume, { path: mount }] of Object.entries(volumes)) {
    const root = mount.replace(/\/+$/, '')
    if (path.startsWith(`${root}/`)) return `/${service}/${volume}/${path.slice(root.length + 1)}`
  }
  return undefined
}

/** `{{ settings.<clé> }}` remplacés par les réglages du serveur ; `undefined` si l'un manque. */
function rendered(template: string, settings: Record<string, unknown>): string | undefined {
  let missing = false
  const text = template.replace(/\{\{\s*settings\.([A-Za-z0-9_]+)\s*\}\}/g, (_, key: string) => {
    const value = settings[key]
    if (value === undefined || value === null) missing = true
    return String(value ?? '')
  })
  return missing || text.includes('{{') ? undefined : text
}

/**
 * Fichiers déclarés par le prefab, traduits en chemins des volumes, puis les fichiers
 * d'inscription de ses emplacements de contenu (`register`), rendus avec les réglages du serveur.
 */
export function declaredFiles(manifest: Manifest | undefined, settings: Record<string, unknown> = {}): DeclaredFile[] {
  if (!manifest) return []
  const files: DeclaredFile[] = Object.entries(manifest.files ?? {}).flatMap(([key, file]) => {
    if (file.editable === false) return []
    const path = volumePath(manifest, file.service, file.path)
    return path ? [{ key, path, format: file.format }] : []
  })
  for (const [slot, content] of Object.entries(manifest.content ?? {})) {
    ;(content.register ?? []).forEach((register, index) => {
      const file = rendered(register.file, settings)
      const path = file && volumePath(manifest, content.service, file)
      // Plusieurs déclarations visent souvent le même fichier : une seule ligne.
      if (path && !files.some((entry) => entry.path === path)) {
        files.push({ key: `register.${slot}.${index}`, path, format: 'json', generated: true })
      }
    })
  }
  return files
}
