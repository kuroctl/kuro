import type { RouteLocationRaw } from "vue-router"

/** Objet qui empêche une action, avec l'endroit où le régler. */
export interface Blocker {
  key: string
  label: string
  detail?: string
  to: RouteLocationRaw
}
