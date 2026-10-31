// État de la mise en route (ADR-0045), lu avant chaque navigation et par la page de connexion
// (nom du projet). Il ne change qu'une fois : gardé tant que le setup ne l'invalide pas.
import { queryOptions, useQuery } from '@tanstack/vue-query'

import { queryClient } from '@/shared/query/client'

import { type SetupStatus, useSetupApi } from './setup.api'

export const setupStatusQuery = queryOptions({
  queryKey: ['setup'],
  queryFn: () => useSetupApi().status(),
  staleTime: Infinity,
})

/** État connu, ou « pas de setup » si l'API ne répond pas : la connexion dira le reste. */
export async function fetchSetupStatus(): Promise<SetupStatus> {
  try {
    return await queryClient.fetchQuery(setupStatusQuery)
  } catch {
    return { needed: false, project_name: null }
  }
}

export function useSetupStatus() {
  return useQuery(setupStatusQuery)
}

export function invalidateSetupStatus() {
  return queryClient.invalidateQueries({ queryKey: setupStatusQuery.queryKey })
}
