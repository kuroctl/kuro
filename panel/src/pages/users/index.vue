<script setup lang="ts">
// Comptes admin (ADR-0007, ADR-0025) : recherche par nom ou e-mail, facettes rôle et état, tri,
// pagination et colonnes affichées, le tout filtré localement (la liste est courte) et gardé dans
// l'URL. Création depuis l'en-tête ; une ligne mène à la fiche (rôles, identifiants, désactivation).
// Les clients relèvent de l'espace client.
import { ShieldCheck, Users, X } from '@lucide/vue'
import { createColumnHelper } from '@tanstack/vue-table'
import { computed, h } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import RoleBadge from '@/features/roles/components/RoleBadge.vue'
import { useRoles } from '@/features/roles/roles.queries'
import CreateAdminDialog from '@/features/users/components/CreateAdminDialog.vue'
import type { AdminUser } from '@/features/users/users.api'
import { useAdminUsers } from '@/features/users/users.queries'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import {
  DataTable,
  DataTableFacetFilter,
  type DataTableFeatures,
  DataTablePagination,
  DataTableSearch,
  DataTableViewOptions,
  type FacetOption,
  sortable,
  useDataTable,
  useTableQuery,
} from '@/shared/ui/data-table'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'

/** Valeur de la facette rôle pour un compte sans rôle. */
const NO_ROLE = 'none'
type Facet = 'role' | 'state'

const { t, locale } = useI18n()
const { can, profile } = usePermissions()
const { data: users, error, isPending } = useAdminUsers(() => can('iam.users.read'))
const { data: roles } = useRoles(() => can('iam.roles.read'))
const { message } = useApiError()

const query = useTableQuery({ sort: 'name', filters: ['role', 'state'] })
const all = computed(() => users.value ?? [])

/** Le compte passe la recherche et les facettes, sauf `except` (pour compter ses valeurs). */
function matches(user: AdminUser, except?: Facet): boolean {
  const needle = query.q.value.toLowerCase()
  if (needle && !user.display_name.toLowerCase().includes(needle) && !user.email.toLowerCase().includes(needle)) return false
  const { role = [], state = [] } = query.filters.value
  if (except !== 'role' && role.length && !role.some((id) => (id === NO_ROLE ? user.roles.length === 0 : user.roles.some((r) => r.id === id)))) {
    return false
  }
  if (except !== 'state' && state.length && !state.includes(user.disabled ? 'disabled' : 'active')) return false
  return true
}

const filtered = computed(() => all.value.filter((user) => matches(user)))

/** Rôles proposés : le catalogue s'il est lisible, sinon ceux portés par les comptes. */
const roleOptions = computed<FacetOption[]>(() => {
  const known = new Map<string, string>()
  for (const role of roles.value ?? []) known.set(role.id, role.name)
  for (const user of all.value) for (const role of user.roles) known.set(role.id, role.name)
  const candidates = all.value.filter((user) => matches(user, 'role'))
  const options = [...known]
    .map(([id, name]) => ({ value: id, label: name, count: candidates.filter((user) => user.roles.some((r) => r.id === id)).length }))
    .sort((a, b) => a.label.localeCompare(b.label))
  if (all.value.some((user) => user.roles.length === 0)) {
    options.push({ value: NO_ROLE, label: t('users.noRole'), count: candidates.filter((user) => user.roles.length === 0).length })
  }
  return options
})

const stateOptions = computed<FacetOption[]>(() => {
  const candidates = all.value.filter((user) => matches(user, 'state'))
  return (['active', 'disabled'] as const).map((state) => ({
    value: state,
    label: t(`users.state.${state}`),
    count: candidates.filter((user) => user.disabled === (state === 'disabled')).length,
  }))
})

const colorOf = (id: string) => roles.value?.find((role) => role.id === id)?.color
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }))

const column = createColumnHelper<DataTableFeatures, AdminUser>()

const columns = column.columns([
  column.accessor('display_name', {
    id: 'name',
    enableHiding: false,
    sortFn: 'text',
    meta: { title: t('users.fields.name') },
    header: sortable(t('users.fields.name')),
    cell: ({ row }) =>
      h('span', [
        h('span', { class: 'flex items-center gap-2 font-medium' }, [
          row.original.display_name,
          row.original.id === profile.value?.user.id ? h(Badge, { variant: 'outline', class: 'font-normal' }, () => t('users.you')) : null,
        ]),
        h('span', { class: 'block text-xs text-muted-foreground' }, row.original.email),
      ]),
  }),
  column.display({
    id: 'roles',
    meta: { title: t('users.fields.roles') },
    header: () => t('users.fields.roles'),
    cell: ({ row }) =>
      h(
        'span',
        { class: 'flex flex-wrap gap-1' },
        row.original.roles.length
          ? row.original.roles.map((role) => h(RoleBadge, { key: role.id, name: role.name, color: colorOf(role.id) }))
          : [h('span', { class: 'text-xs text-muted-foreground' }, t('users.noRole'))],
      ),
  }),
  column.accessor('totp_enabled', {
    id: 'totp',
    meta: { title: t('users.fields.totp'), class: 'w-32' },
    header: sortable(t('users.fields.totp')),
    cell: ({ row }) =>
      row.original.totp_enabled
        ? h('span', { class: 'inline-flex items-center gap-1 text-xs text-success' }, [h(ShieldCheck, { class: 'size-3.5' }), t('users.totp.on')])
        : h('span', { class: 'text-xs text-muted-foreground' }, t('users.totp.off')),
  }),
  column.accessor('disabled', {
    id: 'state',
    meta: { title: t('users.fields.state'), class: 'w-32' },
    header: sortable(t('users.fields.state')),
    cell: ({ row }) =>
      h(Badge, { variant: row.original.disabled ? 'destructive' : 'secondary', class: 'font-normal' }, () =>
        row.original.disabled ? t('users.state.disabled') : t('users.state.active'),
      ),
  }),
  column.accessor((user) => user.last_login_at ?? undefined, {
    id: 'lastLogin',
    sortFn: 'datetime',
    sortUndefined: 'last',
    meta: { title: t('users.fields.lastLogin'), class: 'w-48' },
    header: sortable(t('users.fields.lastLogin')),
    cell: ({ row }) =>
      h('span', { class: 'text-xs text-muted-foreground' },
        row.original.last_login_at ? dates.value.format(new Date(row.original.last_login_at)) : t('users.neverLoggedIn')),
  }),
  column.accessor('created_at', {
    id: 'created',
    sortFn: 'datetime',
    meta: { title: t('users.fields.created'), class: 'w-48' },
    header: sortable(t('users.fields.created')),
    cell: ({ row }) => h('span', { class: 'text-xs text-muted-foreground' }, dates.value.format(new Date(row.original.created_at))),
  }),
])

const { table } = useDataTable({
  data: filtered,
  columns,
  getRowId: (user) => user.id,
  query,
  storageKey: 'users',
  hidden: ['created'],
})
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.users.admins')">
      <CreateAdminDialog v-if="can('iam.users.create')" />
    </PageHeader>
    <div class="grid min-w-0 gap-4 p-6">
      <p v-if="!can('iam.users.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('users.forbidden') }}
      </p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <div v-else-if="isPending" class="grid gap-2"><Skeleton v-for="index in 3" :key="index" class="h-12" /></div>
      <Empty v-else-if="all.length === 0" :icon="Users">{{ t('users.empty') }}</Empty>
      <template v-else>
        <div class="flex flex-wrap items-center gap-2">
          <DataTableSearch v-model="query.q.value" :placeholder="t('users.search')" />
          <DataTableFacetFilter
            :model-value="query.filters.value.role ?? []"
            :title="t('users.fields.roles')"
            :options="roleOptions"
            @update:model-value="(values) => query.setFilter('role', values)"
          />
          <DataTableFacetFilter
            :model-value="query.filters.value.state ?? []"
            :title="t('users.fields.state')"
            :options="stateOptions"
            @update:model-value="(values) => query.setFilter('state', values)"
          />
          <Button v-if="query.active.value" variant="ghost" size="sm" class="h-8" @click="query.reset()">
            {{ t('dataTable.reset') }}<X />
          </Button>
          <DataTableViewOptions :table="table" />
        </div>
        <DataTable :table="table" :row-to="(user) => `/users/${user.id}`" />
        <DataTablePagination :table="table" :query="query" :total="filtered.length" />
      </template>
    </div>
  </div>
</template>
