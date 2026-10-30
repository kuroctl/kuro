import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'
import VueRouter from 'vue-router/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    // Routage par fichiers (`src/pages/`) : doit précéder le plugin Vue.
    VueRouter({ routesFolder: 'src/pages', dts: 'src/typed-router.d.ts' }),
    vue(),
    tailwindcss(),
  ],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
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
