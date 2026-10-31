// Comptes admin, rôles et catalogue des permissions simulés au format de l'API.
import { http, HttpResponse } from 'msw'

import type { Schemas } from '@/shared/api/types'

import { API, profile } from './api'

export const SUPER_ADMIN_ID = '00000000-0000-7000-8000-000000000001'
export const SUPPORT_ID = '01a17f00-0000-7000-8000-0000000000b1'
export const ADMIN_ID = '01a17f00-0000-7000-8000-0000000000c1'

export function role(overrides: Partial<Schemas['RoleDto']> = {}): Schemas['RoleDto'] {
  return {
    id: SUPPORT_ID,
    name: 'support',
    description: 'Lecture des serveurs et du journal.',
    color: '#3b82f6',
    is_system: false,
    permissions: ['servers.read', 'audit.read'],
    members: 1,
    created_at: '2026-10-01T00:00:00Z',
    ...overrides,
  }
}

export const superAdmin = role({
  id: SUPER_ADMIN_ID,
  name: 'super-admin',
  description: 'Toutes les permissions, y compris futures.',
  color: '#64748b',
  is_system: true,
  permissions: ['*'],
})

export function admin(overrides: Partial<Schemas['AdminUserDto']> = {}): Schemas['AdminUserDto'] {
  return {
    id: ADMIN_ID,
    email: 'alice@example.com',
    display_name: 'Alice',
    totp_enabled: true,
    disabled: false,
    password_change_required: false,
    invitation_pending: false,
    last_login_at: '2026-10-27T09:00:00Z',
    created_at: '2026-10-01T00:00:00Z',
    roles: [{ id: SUPPORT_ID, name: 'support' }],
    ...overrides,
  }
}

/** Le compte connecté, tel que l'API le liste. */
export const me: Schemas['AdminUserDto'] = { ...profile.user, last_login_at: null }

export const catalog: Schemas['CategoryDto'][] = [
  {
    key: 'iam',
    label: 'Accès et sécurité',
    permissions: [
      { key: 'iam.users.read', label: '', sensitive: false, availability: 'available' },
      { key: 'iam.roles.manage', label: '', sensitive: true, availability: 'available' },
    ],
  },
  {
    key: 'servers',
    label: 'Serveurs',
    permissions: [
      { key: 'servers.read', label: '', sensitive: false, availability: 'available' },
      { key: 'servers.power', label: '', sensitive: false, availability: 'available' },
      { key: 'servers.transfer', label: '', sensitive: false, availability: 'reserved' },
    ],
  },
  { key: 'audit', label: 'Journal d’audit', permissions: [{ key: 'audit.read', label: '', sensitive: false, availability: 'available' }] },
]

export const iam = () => [
  http.get(`${API}/users`, () => HttpResponse.json([me, admin()])),
  http.get(`${API}/users/${ADMIN_ID}`, () => HttpResponse.json(admin())),
  http.get(`${API}/roles`, () => HttpResponse.json([superAdmin, role()])),
  http.get(`${API}/roles/${SUPPORT_ID}`, () => HttpResponse.json(role())),
  http.get(`${API}/roles/${SUPER_ADMIN_ID}`, () => HttpResponse.json(superAdmin)),
  http.get(`${API}/permissions`, () => HttpResponse.json(catalog)),
]
