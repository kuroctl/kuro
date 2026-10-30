// Monte l'application complète (mêmes plugins que `main.ts`) sur un historique en mémoire.
import { render } from '@testing-library/vue'
import userEvent from '@testing-library/user-event'
import { createMemoryHistory, type Router } from 'vue-router'

import App from '@/App.vue'
import { installApp } from '@/bootstrap'

export async function renderApp(path: string) {
  const history = createMemoryHistory()
  history.replace(path)
  let router!: Router
  const view = render(App, {
    global: { plugins: [{ install: (app) => (router = installApp(app, history)) }] },
  })
  await router.isReady()
  return { ...view, router, user: userEvent.setup() }
}
