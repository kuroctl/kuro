/// <reference types="vite/client" />
/// <reference types="vue-router/auto-routes" />

import 'vue-router'

declare module 'vue-router' {
  interface RouteMeta {
    /** Sous-menu contextuel à la place de celui de la section. */
    subnav?: 'server'
  }
}
