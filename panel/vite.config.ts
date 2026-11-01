import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'
import VueRouter from 'vue-router/vite'

import { kuroPanelExtensions, parseExtensions, routesFolders } from './build/extensions'

const root = fileURLToPath(new URL('.', import.meta.url))
// Extensions du panel admin (ADR-0051 § 8) : `KURO_PANEL_EXTENSIONS=<nom>=<dossier>,…`. Les
// tests montent l'extension de démonstration de `tests/fixtures/panel-extension`.
const extensions = parseExtensions(
  process.env.VITEST ? 'demo=tests/fixtures/panel-extension' : process.env.KURO_PANEL_EXTENSIONS,
  root,
)

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    // Routage par fichiers (`src/pages/`, puis les pages des extensions sous `/<nom>/`) : doit
    // précéder le plugin Vue. Les types des routes ne s'écrivent que pour le panel seul (ils sont
    // commités) ; un build avec extensions choisit leur emplacement (`KURO_PANEL_TYPED_ROUTER`).
    VueRouter({
      routesFolder: routesFolders(extensions),
      dts: extensions.length === 0 ? 'src/typed-router.d.ts' : (process.env.KURO_PANEL_TYPED_ROUTER ?? false),
    }),
    vue(),
    tailwindcss(),
    kuroPanelExtensions(extensions),
  ],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    // Les dossiers des extensions sont hors du panel : le serveur de dev doit pouvoir les servir.
    fs: { allow: [root, ...extensions.map((extension) => extension.dir)] },
    // Même origine que l'API : le cookie de rafraîchissement `SameSite=Strict` fonctionne.
    // `KURO_API_URL` : API sur un autre port (plusieurs environnements en parallèle).
    // `ws` : WebSocket de la console, relayé lui aussi (même origine, `Origin` accepté par l'API).
    proxy: {
      '/api': { target: process.env.KURO_API_URL ?? 'http://127.0.0.1:8080', ws: true },
      // Serveur MCP et son serveur d'autorisation (ADR-0023), sur l'origine du panel en dev.
      '^/(mcp|oauth/(authorize|token|register|revoke)|\.well-known/oauth-)': {
        target: process.env.KURO_API_URL ?? 'http://127.0.0.1:8080',
      },
    },
  },
  test: {
    environment: 'jsdom',
    include: ['tests/**/*.spec.ts'],
    setupFiles: ['./tests/setup.ts'],
    // Le premier test d'un fichier charge l'application à froid (éditeur compris) : sur un
    // runner de CI lent, 5 s ne suffisent pas toujours.
    testTimeout: 15_000,
  },
})
