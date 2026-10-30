// Branchement de l'application, partagé par `main.ts` et les tests de features : les tests
// montent ainsi exactement la même application (seul l'historique du routeur change).
import { VueQueryPlugin } from '@tanstack/vue-query'
import { createPinia } from 'pinia'
import type { App } from 'vue'
import type { RouterHistory } from 'vue-router'

import { connectSessionToHttp } from '@/features/auth/session.store'
import { applyLocale, i18n, type Locale } from '@/shared/i18n'
import { queryClient } from '@/shared/query/client'
import { createAppRouter } from '@/shared/router'

export function installApp(app: App, history?: RouterHistory) {
  const router = createAppRouter(history)
  app.use(createPinia())
  app.use(i18n)
  app.use(VueQueryPlugin, { queryClient })
  app.use(router)

  applyLocale(i18n.global.locale.value as Locale)
  connectSessionToHttp(() => {
    void router.push({ path: '/login', query: { redirect: router.currentRoute.value.fullPath } })
  })
  return router
}
