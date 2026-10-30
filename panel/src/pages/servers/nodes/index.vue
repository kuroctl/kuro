<script setup lang="ts">
// Liste des nœuds (ADR-0025), en mode local : la liste est courte, elle est filtrée ici
// (recherche, facettes état et région) puis triée et paginée par la table. Toute la vue tient
// dans l'URL ; une ligne mène à la fiche du nœud. « Ajouter un nœud » donne son ticket d'enrôlement.
import { HardDrive, X } from '@lucide/vue'
import { createColumnHelper } from '@tanstack/vue-table'
import { computed, h } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import CreateNodeDialog from '@/features/nodes/components/CreateNodeDialog.vue'
import type { Node } from '@/features/nodes/nodes.api'
import { useNodes } from '@/features/nodes/nodes.queries'
import { NODE_STATUSES, nodeBadge } from '@/features/nodes/status'
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

const { t } = useI18n()
const { data: nodes, isPending } = useNodes()
const { can } = usePermissions()

/** Valeur de facette d'un nœud sans région : un identifiant de région n'a jamais de `_`. */
const NO_REGION = '_none'

const query = useTableQuery({ sort: 'name', filters: ['status', 'region'] })

const label = (node: Node) => node.display_name ?? node.name
const regionOf = (node: Node) => node.region?.slug ?? NO_REGION

/** Recherche sur le libellé et le nom technique. */
function matchesSearch(node: Node) {
  const needle = query.q.value.toLowerCase()
  return !needle || label(node).toLowerCase().includes(needle) || node.name.toLowerCase().includes(needle)
}
function matches(node: Node, skip?: 'status' | 'region') {
  const { status = [], region = [] } = query.filters.value
  return (
    matchesSearch(node) &&
    (skip === 'status' || !status.length || status.includes(node.status)) &&
    (skip === 'region' || !region.length || region.includes(regionOf(node)))
  )
}

const all = computed(() => nodes.value ?? [])
const filtered = computed(() => all.value.filter((node) => matches(node)))

/** Nombre de nœuds par valeur, les autres filtres appliqués. */
function count(skip: 'status' | 'region', value: (node: Node) => string) {
  const counts = new Map<string, number>()
  for (const node of all.value) if (matches(node, skip)) counts.set(value(node), (counts.get(value(node)) ?? 0) + 1)
  return counts
}

// États connus d'abord, dans leur ordre ; seuls ceux présents dans la liste (ou cochés).
const statusOptions = computed<FacetOption[]>(() => {
  const counts = count('status', (node) => node.status)
  const present = new Set([...all.value.map((node) => node.status), ...(query.filters.value.status ?? [])])
  const known: readonly string[] = NODE_STATUSES
  return [...known.filter((s) => present.has(s)), ...[...present].filter((s) => !known.includes(s))].map((status) => ({
    value: status,
    label: t(`nodes.status.${status}`),
    count: counts.get(status) ?? 0,
  }))
})

const regionOptions = computed<FacetOption[]>(() => {
  const counts = count('region', regionOf)
  const names = new Map<string, string>()
  for (const node of all.value) if (node.region) names.set(node.region.slug, node.region.name)
  const options: FacetOption[] = [...names]
    .sort(([, a], [, b]) => a.localeCompare(b))
    .map(([slug, name]) => ({ value: slug, label: name, count: counts.get(slug) ?? 0 }))
  if (all.value.some((node) => !node.region)) options.push({ value: NO_REGION, label: t('nodes.list.noRegion'), count: counts.get(NO_REGION) ?? 0 })
  return options
})

const column = createColumnHelper<DataTableFeatures, Node>()

const columns = column.columns([
  column.accessor(label, {
    id: 'name',
    enableHiding: false,
    meta: { title: t('nodes.fields.name') },
    header: sortable(t('nodes.fields.name')),
    cell: ({ row }) =>
      h('span', { class: 'font-medium' }, [
        label(row.original),
        row.original.display_name ? h('span', { class: 'ml-1 font-mono text-xs text-muted-foreground' }, row.original.name) : null,
      ]),
  }),
  column.accessor('status', {
    id: 'status',
    meta: { title: t('nodes.fields.status') },
    header: sortable(t('nodes.fields.status')),
    cell: ({ row }) =>
      h(Badge, { variant: nodeBadge[row.original.status] ?? 'secondary' }, () => [
        h('span', { class: 'size-[7px] rounded-full bg-current', 'aria-hidden': 'true' }),
        t(`nodes.status.${row.original.status}`),
      ]),
  }),
  // Chaîne vide plutôt qu'`undefined` : les nœuds sans région se trient avec les autres.
  column.accessor((node) => node.region?.name ?? '', {
    id: 'region',
    meta: { title: t('nodes.fields.region') },
    header: sortable(t('nodes.fields.region')),
  }),
  column.display({
    id: 'host',
    meta: { title: t('nodes.fields.host'), class: 'text-xs text-muted-foreground' },
    header: () => t('nodes.fields.host'),
    cell: ({ row }) => (row.original.host ? t('nodes.hostSummary', { os: row.original.host.os, cores: row.original.host.cpu_cores }) : null),
  }),
  column.accessor('created_at', {
    id: 'created',
    meta: { title: t('nodes.fields.created') },
    header: sortable(t('nodes.fields.created')),
    cell: ({ row }) => h('span', { class: 'text-xs text-muted-foreground' }, new Date(row.original.created_at).toLocaleDateString()),
  }),
])

const { table } = useDataTable({
  data: filtered,
  columns,
  getRowId: (node) => node.id,
  query,
  storageKey: 'nodes',
  hidden: ['created'],
})
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('nodes.title')">
      <CreateNodeDialog v-if="can('nodes.create')" />
    </PageHeader>
    <div class="grid min-w-0 gap-4 p-6">
      <div v-if="isPending" class="grid gap-2"><Skeleton v-for="index in 3" :key="index" class="h-12" /></div>
      <Empty v-else-if="!all.length" :icon="HardDrive">{{ t('nodes.empty') }}</Empty>
      <template v-else>
        <div class="flex flex-wrap items-center gap-2">
          <DataTableSearch v-model="query.q.value" :placeholder="t('nodes.list.search')" />
          <DataTableFacetFilter
            :model-value="query.filters.value.status ?? []"
            :title="t('nodes.fields.status')"
            :options="statusOptions"
            @update:model-value="(values) => query.setFilter('status', values)"
          />
          <DataTableFacetFilter
            :model-value="query.filters.value.region ?? []"
            :title="t('nodes.fields.region')"
            :options="regionOptions"
            @update:model-value="(values) => query.setFilter('region', values)"
          />
          <Button v-if="query.active.value" variant="ghost" size="sm" class="h-8" @click="query.reset()">
            {{ t('dataTable.reset') }}<X />
          </Button>
          <DataTableViewOptions :table="table" />
        </div>
        <DataTable :table="table" :row-to="(node) => `/servers/nodes/${node.id}`" />
        <DataTablePagination :table="table" :query="query" :total="filtered.length" />
      </template>
    </div>
  </div>
</template>
