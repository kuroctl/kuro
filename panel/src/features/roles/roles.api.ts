// Appels API des rôles et du catalogue des permissions (ADR-0007). Aucune logique ici : les
// erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/roles'

export type Role = Schemas['RoleDto']
export type PermissionCategory = Schemas['CategoryDto']
export type PermissionKey = Schemas['PermissionKey']

export function useRolesApi() {
  return {
    list: () => http.get<Role[]>(BASE).then((response) => response.data),

    get: (id: string) => http.get<Role>(`${BASE}/${id}`).then((response) => response.data),

    create: (body: Schemas['CreateRoleRequest']) => http.post<Role>(BASE, body).then((response) => response.data),

    update: (id: string, body: Schemas['UpdateRoleRequest']) =>
      http.patch<Role>(`${BASE}/${id}`, body).then((response) => response.data),

    remove: (id: string) => http.delete<void>(`${BASE}/${id}`).then(() => undefined),

    catalog: () => http.get<PermissionCategory[]>('/admin/v1/permissions').then((response) => response.data),
  }
}
