// Noms publiés sous la zone (ADR-0026), composés comme le fait l'API.
import type { PublicZone } from './dns.api'

/** `<nœud>.<sous-domaine des nœuds>.<base>`, ou `null` sans DNS. */
export function nodeHost(zone: PublicZone | undefined, node: string): string | null {
  if (!zone?.base_domain || !zone.nodes_label) return null
  return `${node}.${zone.nodes_label}.${zone.base_domain}`
}

/** `<label>.<base>`, ou `null` sans DNS ou sans nom. */
export function serverHost(zone: PublicZone | undefined, label: string | null | undefined): string | null {
  if (!zone?.base_domain || !label) return null
  return `${label}.${zone.base_domain}`
}

/** `https://<identifiant>.<domaine des sites>` (ADR-0038), ou `null` sans domaine des sites ou sans site. */
export function siteUrl(zone: PublicZone | undefined, webId: string | null | undefined): string | null {
  if (!zone?.sites_domain || !webId) return null
  return `https://${webId}.${zone.sites_domain}`
}
