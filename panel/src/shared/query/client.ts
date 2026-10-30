// Cache des lectures (TanStack Vue Query). Les écritures passent directement par les
// composables `*.api.ts`, puis invalident les requêtes concernées.
import { QueryClient } from '@tanstack/vue-query'

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      // Une erreur d'API ne se corrige pas en réessayant : l'utilisateur la voit tout de suite.
      retry: false,
      refetchOnWindowFocus: true,
    },
  },
})
