// Sortie du panel vers une autre origine (retour d'un client OAuth). Isolé pour être simulé
// dans les tests, où la navigation réelle n'existe pas.
export function leaveTo(url: string) {
  window.location.assign(url)
}
