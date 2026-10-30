<script setup lang="ts">
// Enregistrements créés par Kuro (ADR-0026), en mode local : la liste est courte, elle est
// filtrée ici (recherche, facettes type et statut) puis triée et paginée par la table. Le
// propriétaire est nommé d'après les listes de nœuds et de serveurs quand l'admin peut les lire.
import { Globe, X } from '@lucide/vue'
import { useQuery } from '@tanstack/vue-query'
import { createColumnHelper } from '@tanstack/vue-table'
import { computed, h } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import { nodesQuery } from '@/features/nodes/nodes.queries'
import { serversQuery } from '@/features/servers/servers.queries'
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
  plain,
  sortable,
  useDataTable,
  useTableQuery,
} from '@/shared/ui/data-table'
import { Empty } from '@/shared/ui/empty'

import type { DnsRecord } from '../dns.api'

type Status = 'published' | 'error' | 'pending'
const KINDS = ['A', 'AAAA', 'CNAME', 'SRV']
const STATUSES: Status[] = ['published', 'error', 'pending']

const props = defineProps<{ records: DnsRecord[] }>()
const { t } = useI18n()
const { can } = usePermissions()

// Listes lues seulement si l'admin y a droit : sinon le propriétaire reste anonyme.
const nodesEnabled = computed(() => can('nodes.read'))
const serversEnabled = computed(() => can('servers.read'))
const { data: nodes } = useQuery(computed(() => ({ ...nodesQuery, enabled: nodesEnabled.value })))
const { data: servers } = useQuery(computed(() => ({ ...serversQuery, enabled: serversEnabled.value })))

const query = useTableQuery({ sort: 'name', filters: ['kind', 'status'] })

/** Une erreur prime : un enregistrement publié peut avoir échoué à sa dernière mise à jour. */
const statusOf = (record: DnsRecord): Status => (record.error ? 'error' : record.published ? 'published' : 'pending')

function ownerName(record: DnsRecord): string {
  if (record.owner_kind === 'node' && nodesEnabled.value) {
    const node = nodes.value?.find((n) => n.id === record.owner_id)
    if (node) return node.display_name ?? node.name
  }
  if (record.owner_kind === 'server' && serversEnabled.value) {
    const server = servers.value?.find((s) => s.id === record.owner_id)
    if (server) return server.name
  }
  return record.owner_id.slice(0, 8)
}

function matches(record: DnsRecord, skip?: 'kind' | 'status') {
  const { kind = [], status = [] } = query.filters.value
  const needle = query.q.value.toLowerCase()
  return (
    (!needle ||
      record.name.toLowerCase().includes(needle) ||
      record.content.toLowerCase().includes(needle) ||
      ownerName(record).toLowerCase().includes(needle)) &&
    (skip === 'kind' || !kind.length || kind.includes(record.kind)) &&
    (skip === 'status' || !status.length || status.includes(statusOf(record)))
  )
}

const filtered = computed(() => props.records.filter((record) => matches(record)))

function options(skip: 'kind' | 'status', known: readonly string[], value: (record: DnsRecord) => string, label: (v: string) => string) {
  const counts = new Map<string, number>()
  for (const record of props.records) if (matches(record, skip)) counts.set(value(record), (counts.get(value(record)) ?? 0) + 1)
  const present = new Set([...props.records.map(value), ...(query.filters.value[skip] ?? [])])
  return [...known.filter((v) => present.has(v)), ...[...present].filter((v) => !known.includes(v))].map<FacetOption>((v) => ({
    value: v,
    label: label(v),
    count: counts.get(v) ?? 0,
  }))
}
const kindOptions = computed(() => options('kind', KINDS, (r) => r.kind, (v) => v))
const statusOptions = computed(() => options('status', STATUSES, statusOf, (v) => t(`dns.records.status.${v}`)))

const column = createColumnHelper<DataTableFeatures, DnsRecord>()

const columns = column.columns([
  column.accessor('kind', {
    id: 'kind',
    meta: { title: t('dns.records.fields.kind'), class: 'w-24' },
    header: sortable(t('dns.records.fields.kind')),
    cell: ({ row }) => h(Badge, { variant: 'outline', class: 'font-mono' }, () => row.original.kind),
  }),
  column.accessor('name', {
    id: 'name',
    enableHiding: false,
    meta: { title: t('dns.records.fields.name'), class: 'font-mono text-xs' },
    header: sortable(t('dns.records.fields.name')),
  }),
  column.accessor('content', {
    id: 'content',
    meta: { title: t('dns.records.fields.content'), class: 'font-mono text-xs' },
    header: sortable(t('dns.records.fields.content')),
  }),
  column.accessor(ownerName, {
    id: 'owner',
    meta: { title: t('dns.records.fields.owner') },
    header: sortable(t('dns.records.fields.owner')),
    cell: ({ row }) =>
      h('span', [
        h('span', ownerName(row.original)),
        h('span', { class: 'ml-1 text-xs text-muted-foreground' }, t(`dns.records.owner.${row.original.owner_kind === 'node' ? 'node' : 'server'}`)),
      ]),
  }),
  column.display({
    id: 'status',
    meta: { title: t('dns.records.fields.status') },
    header: plain(t('dns.records.fields.status')),
    cell: ({ row }) => {
      const status = statusOf(row.original)
      const variant = status === 'error' ? 'destructive' : status === 'published' ? 'success' : 'secondary'
      return h('div', { class: 'grid gap-1' }, [
        h(Badge, { variant }, () => t(`dns.records.status.${status}`)),
        row.original.error ? h('span', { class: 'text-xs text-destructive' }, row.original.error) : null,
      ])
    },
  }),
  column.accessor((record) => record.synced_at ?? '', {
    id: 'synced',
    meta: { title: t('dns.records.fields.synced') },
    header: sortable(t('dns.records.fields.synced')),
    cell: ({ row }) =>
      h('span', { class: 'text-xs text-muted-foreground' }, row.original.synced_at ? new Date(row.original.synced_at).toLocaleString() : '—'),
  }),
])

const { table } = useDataTable({
  data: filtered,
  columns,
  getRowId: (record) => record.id,
  query,
  storageKey: 'dns-records',
  hidden: ['synced'],
})
</script>

<template>
  <Empty v-if="!records.length" :icon="Globe">{{ t('dns.records.empty') }}</Empty>
  <div v-else class="grid min-w-0 gap-4">
    <div class="flex flex-wrap items-center gap-2">
      <DataTableSearch v-model="query.q.value" :placeholder="t('dns.records.search')" />
      <DataTableFacetFilter
        :model-value="query.filters.value.kind ?? []"
        :title="t('dns.records.fields.kind')"
        :options="kindOptions"
        @update:model-value="(values) => query.setFilter('kind', values)"
      />
      <DataTableFacetFilter
        :model-value="query.filters.value.status ?? []"
        :title="t('dns.records.fields.status')"
        :options="statusOptions"
        @update:model-value="(values) => query.setFilter('status', values)"
      />
      <Button v-if="query.active.value" variant="ghost" size="sm" class="h-8" @click="query.reset()">
        {{ t('dataTable.reset') }}<X />
      </Button>
      <DataTableViewOptions :table="table" />
    </div>
    <DataTable :table="table" />
    <DataTablePagination :table="table" :query="query" :total="filtered.length" />
  </div>
</template>
