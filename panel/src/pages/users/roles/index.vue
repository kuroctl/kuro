<script setup lang="ts">
// Rôles de l'espace admin (ADR-0007, ADR-0025) : badge, description, membres, permissions.
// Recherche, tri et pagination locaux (la liste est courte), gardés dans l'URL. Création depuis
// l'en-tête ; une ligne mène à l'éditeur du rôle.
import { Lock, Plus, ShieldCheck, X } from '@lucide/vue'
import { createColumnHelper } from '@tanstack/vue-table'
import { computed, h } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { usePermissions } from '@/features/account/account.queries'
import RoleBadge from '@/features/roles/components/RoleBadge.vue'
import type { Role } from '@/features/roles/roles.api'
import { useRoles } from '@/features/roles/roles.queries'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { DataTable, type DataTableFeatures, DataTablePagination, DataTableSearch, DataTableViewOptions, plain, sortable, useDataTable, useTableQuery } from '@/shared/ui/data-table'
import { Skeleton } from '@/shared/ui/skeleton'

const { t, locale } = useI18n()
const router = useRouter()
const { can } = usePermissions()
const { data: roles, error, isPending } = useRoles(() => can('iam.roles.read'))
const { message } = useApiError()

const query = useTableQuery({ sort: 'name' })

/** Recherche dans le nom et la description. */
const filtered = computed(() => {
  const needle = query.q.value.toLowerCase()
  return (roles.value ?? []).filter(
    (role) => !needle || role.name.toLowerCase().includes(needle) || role.description.toLowerCase().includes(needle),
  )
})

const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium' }))
const column = createColumnHelper<DataTableFeatures, Role>()

const columns = column.columns([
  column.accessor('name', {
    id: 'name',
    enableHiding: false,
    sortFn: 'text',
    meta: { title: t('roles.fields.name') },
    header: sortable(t('roles.fields.name')),
    cell: ({ row }) =>
      h('span', { class: 'flex items-center gap-2' }, [
        h(RoleBadge, { name: row.original.name, color: row.original.color }),
        row.original.is_system
          ? h(Badge, { variant: 'outline', class: 'gap-1 font-normal' }, () => [h(Lock), t('roles.system')])
          : null,
      ]),
  }),
  column.accessor('description', {
    id: 'description',
    enableSorting: false,
    meta: { title: t('roles.fields.description'), class: 'text-sm text-muted-foreground' },
    header: plain(t('roles.fields.description')),
  }),
  column.accessor('members', {
    id: 'members',
    meta: { title: t('roles.fields.members'), class: 'w-32 text-right tabular-nums' },
    header: sortable(t('roles.fields.members')),
  }),
  // « Toutes » (`*`) se classe au-dessus de tout décompte.
  column.accessor((role) => (role.permissions.includes('*') ? Number.MAX_SAFE_INTEGER : role.permissions.length), {
    id: 'permissions',
    sortFn: 'basic',
    meta: { title: t('roles.fields.permissions'), class: 'w-36 text-right tabular-nums' },
    header: sortable(t('roles.fields.permissions')),
    cell: ({ row }) =>
      row.original.permissions.includes('*')
        ? h('span', { class: 'inline-flex items-center gap-1 text-xs' }, [h(ShieldCheck, { class: 'size-3.5' }), t('roles.all')])
        : String(row.original.permissions.length),
  }),
  column.accessor('created_at', {
    id: 'created',
    sortFn: 'datetime',
    meta: { title: t('roles.fields.created'), class: 'w-40' },
    header: sortable(t('roles.fields.created')),
    cell: ({ row }) => h('span', { class: 'text-xs text-muted-foreground' }, dates.value.format(new Date(row.original.created_at))),
  }),
])

const { table } = useDataTable({
  data: filtered,
  columns,
  getRowId: (role) => role.id,
  query,
  storageKey: 'roles',
  hidden: ['created'],
})
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.users.roles')">
      <Button v-if="can('iam.roles.manage')" size="sm" @click="router.push('/users/roles/new')"><Plus />{{ t('roles.create.action') }}</Button>
    </PageHeader>
    <div class="grid min-w-0 gap-4 p-6">
      <p v-if="!can('iam.roles.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t('roles.forbidden') }}</p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <div v-else-if="isPending" class="grid gap-2"><Skeleton v-for="index in 3" :key="index" class="h-12" /></div>
      <template v-else>
        <div class="flex flex-wrap items-center gap-2">
          <DataTableSearch v-model="query.q.value" :placeholder="t('roles.search')" />
          <Button v-if="query.active.value" variant="ghost" size="sm" class="h-8" @click="query.reset()">
            {{ t('dataTable.reset') }}<X />
          </Button>
          <DataTableViewOptions :table="table" />
        </div>
        <DataTable :table="table" :row-to="(role) => `/users/roles/${role.id}`" />
        <DataTablePagination :table="table" :query="query" :total="filtered.length" />
      </template>
    </div>
  </div>
</template>
