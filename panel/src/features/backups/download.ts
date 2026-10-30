// Téléchargement d'une sauvegarde : le ticket part dans le corps d'un formulaire POST, jamais dans
// l'URL. Le navigateur télécharge lui-même (barre de progression, pas de copie en mémoire).
export function submitDownload(server: string, backup: string, ticket: string) {
  const form = document.createElement('form')
  form.method = 'POST'
  form.action = `/api/admin/v1/servers/${server}/backups/${backup}/download`
  form.hidden = true
  const input = document.createElement('input')
  input.type = 'hidden'
  input.name = 'ticket'
  input.value = ticket
  form.append(input)
  document.body.append(form)
  form.submit()
  form.remove()
}
