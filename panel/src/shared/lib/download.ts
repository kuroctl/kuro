// Enregistrement d'un fichier reçu par l'API (archive, export) : le navigateur propose le fichier
// sous le nom donné, depuis une URL locale aussitôt révoquée.
export function saveBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.hidden = true
  document.body.append(anchor)
  anchor.click()
  anchor.remove()
  URL.revokeObjectURL(url)
}
