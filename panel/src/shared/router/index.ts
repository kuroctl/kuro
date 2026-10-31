// Routeur : routes générées depuis `src/pages/` (Vue Router 5, routage par fichiers).
import { createRouter, createWebHistory, type RouterHistory } from 'vue-router'
import { handleHotUpdate, routes } from 'vue-router/auto-routes'

import { useSessionStore } from '@/features/auth/session.store'
import { fetchSetupStatus } from '@/features/setup/setup.queries'

/** Routes accessibles sans session, affichées hors de la coque du panel. */
export const PUBLIC_PATHS = ['/login', '/setup']

/** Routes qui exigent une session mais s'affichent hors de la coque (consentement OAuth). */
export const BARE_PATHS = ['/oauth/consent']

/** Un routeur par application (les tests en créent un neuf, en mémoire). */
export function createAppRouter(history: RouterHistory = createWebHistory()) {
  const router = createRouter({ history, routes })

  router.beforeEach(async (to) => {
    // Première mise en route (ADR-0045) : tant que l'API n'a aucun compte, tout mène à l'assistant.
    const setup = await fetchSetupStatus()
    if (setup.needed && to.path !== '/setup') return { path: '/setup' }
    if (!setup.needed && to.path === '/setup') return { path: '/login' }
    if (PUBLIC_PATHS.includes(to.path)) return true

    const session = useSessionStore()
    await session.restore()
    if (!session.authenticated) {
      return { path: '/login', query: { redirect: to.fullPath } }
    }
    return true
  })

  if (import.meta.hot) handleHotUpdate(router)
  return router
}
