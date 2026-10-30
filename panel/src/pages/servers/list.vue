<script setup lang="ts">
// Liste des serveurs (ADR-0025) : recherche, filtres rapides par jeu, facettes (état, nœud,
// mise à jour disponible, suspendus), tri et pagination par l'API, sélection étendue à tous les
// résultats et actions groupées. Toute la vue tient dans l'URL.
import { ArrowUpCircle, Server as ServerIcon, X } from '@lucide/vue'
import { createColumnHelper } from '@tanstack/vue-table'
import { computed, h, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { useDnsZone } from '@/features/dns/dns.queries'
import { serverHost } from '@/features/dns/hostnames'
import PrefabIcon from '@/features/prefabs/components/PrefabIcon.vue'
import CreateServerDialog from '@/features/servers/components/CreateServerDialog.vue'
import ServerBulkActions from '@/features/servers/components/ServerBulkActions.vue'
import ServerStateBadge from '@/features/servers/components/ServerStateBadge.vue'
import { type Server, useServersApi } from '@/features/servers/servers.api'
import { useServerSearch } from '@/features/servers/servers.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Checkbox } from '@/shared/ui/checkbox'
import {
  DataTable,
  DataTableFacetFilter,
  type DataTableFeatures,
  DataTablePagination,
  DataTableSearch,
  DataTableSelectionBar,
  DataTableViewOptions,
  sortable,
  useDataTable,
  useTableQuery,
} from '@/shared/ui/data-table'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'

const { t } = useI18n()
/** Zone DNS : un serveur nommé s'affiche `<label>.<base>:<port>` (ADR-0026). */
const { data: zone } = useDnsZone()

const query = useTableQuery({ sort: 'name', filters: ['prefab', 'node', 'state', 'upgrade', 'suspended'] })
const params = computed(() => ({
  ...query.params.value,
  upgrade: query.filters.value.upgrade?.[0],
  suspended: query.filters.value.suspended?.[0],
}))
const { data: page, isPending } = useServerSearch(params)
const items = computed(() => page.value?.items ?? [])
const total = computed(() => page.value?.total ?? 0)

const column = createColumnHelper<DataTableFeatures, Server>()

const columns = column.columns([
  column.display({
    id: 'select',
    enableHiding: false,
    meta: { class: 'w-10' },
    header: ({ table }) =>
      h(Checkbox, {
        modelValue: table.getIsAllPageRowsSelected() || (table.getIsSomePageRowsSelected() && 'indeterminate'),
        'onUpdate:modelValue': (value: boolean | 'indeterminate') => table.toggleAllPageRowsSelected(value === true),
        'aria-label': t('dataTable.selectPage'),
      }),
    cell: ({ row }) =>
      h(Checkbox, {
        modelValue: row.getIsSelected(),
        'onUpdate:modelValue': (value: boolean | 'indeterminate') => row.toggleSelected(value === true),
        'aria-label': t('dataTable.selectRow', { name: row.original.name }),
      }),
  }),
  column.accessor('name', {
    id: 'name',
    enableHiding: false,
    meta: { title: t('servers.fields.name') },
    header: sortable(t('servers.fields.name')),
    cell: ({ row }) => h('span', { class: 'font-medium' }, row.original.name),
  }),
  column.accessor('state', {
    id: 'state',
    meta: { title: t('servers.fields.state') },
    header: sortable(t('servers.fields.state')),
    cell: ({ row }) =>
      h('span', { class: 'flex flex-wrap items-center gap-1.5' }, [
        h(ServerStateBadge, { state: row.original.state, suspended: !!row.original.suspended_at }),
        // Changements de contenu en attente du prochain démarrage.
        row.original.restart_required
          ? h(Badge, { variant: 'warning', class: 'font-normal', title: t('servers.pending.title') }, () => t('servers.pending.pill'))
          : null,
      ]),
  }),
  column.accessor((server) => server.prefab.id, {
    id: 'prefab',
    meta: { title: t('servers.fields.prefab') },
    header: sortable(t('servers.fields.prefab')),
    cell: ({ row }) => {
      const upgrade = page.value?.upgrades[row.original.id]
      return h('span', { class: 'flex items-center gap-2' }, [
        h(PrefabIcon, { prefab: row.original.prefab, class: 'size-6 rounded-md text-xs' }),
        h('span', { class: 'font-mono text-xs' }, `${row.original.prefab.id} · ${row.original.prefab.version}`),
        upgrade
          ? h(Badge, { variant: 'outline', class: 'gap-1 border-info/40 text-info', title: t('servers.list.upgradeTo', { version: upgrade }) }, () => [
              h(ArrowUpCircle, { class: 'size-3' }),
              upgrade,
            ])
          : null,
      ])
    },
  }),
  column.accessor((server) => server.node.name, {
    id: 'node',
    meta: { title: t('servers.fields.node') },
    header: sortable(t('servers.fields.node')),
  }),
  column.display({
    id: 'address',
    meta: { title: t('servers.fields.address'), class: 'font-mono text-xs' },
    header: () => t('servers.fields.address'),
    cell: ({ row }) => {
      const host = serverHost(zone.value, row.original.dns_label)
      return h(
        'span',
        row.original.allocations.map((a) =>
          h('span', { class: 'block', key: a.name, title: host ? `${a.ip}:${a.port}` : undefined }, `${host ?? a.ip}:${a.port}`),
        ),
      )
    },
  }),
  column.accessor('memory_mb', {
    id: 'memory',
    meta: { title: t('servers.fields.resources'), class: 'text-right' },
    header: sortable(t('servers.fields.resources')),
    cell: ({ row }) =>
      h('span', { class: 'text-xs text-muted-foreground tabular-nums' },
        t('servers.resourcesSummary', { memory: row.original.memory_mb, cpu: row.original.cpu_percent })),
  }),
  column.accessor('created_at', {
    id: 'created',
    meta: { title: t('servers.fields.created') },
    header: sortable(t('servers.fields.created')),
    cell: ({ row }) => h('span', { class: 'text-xs text-muted-foreground' }, new Date(row.original.created_at).toLocaleDateString()),
  }),
])

const { table, selectedIds, clearSelection } = useDataTable({
  data: items,
  columns,
  getRowId: (server) => server.id,
  query,
  rowCount: total,
  storageKey: 'servers',
  hidden: ['created'],
})

/** Sélection étendue à tous les résultats du filtre. */
const allMatching = ref(false)
watch(params, () => {
  allMatching.value = false
  clearSelection()
})
function clear() {
  allMatching.value = false
  clearSelection()
}
const pageSelected = computed(() => items.value.length > 0 && selectedIds.value.length === items.value.length)
const targetCount = computed(() => (allMatching.value ? Math.min(total.value, 1000) : selectedIds.value.length))
const names = computed(() => Object.fromEntries(items.value.map((server) => [server.id, server.name])))

async function resolve(): Promise<string[]> {
  if (!allMatching.value) return selectedIds.value
  const result = await useServersApi().search({ ...params.value, page: 1, per_page: 1, all_ids: true })
  return result.ids ?? []
}

const facets = computed(() => page.value?.facets)
const games = computed(() => facets.value?.prefab ?? [])
const stateOptions = computed(() =>
  (facets.value?.state ?? []).map((f) => ({ value: f.value, label: t(`servers.state.${f.value}`), count: f.count })),
)
const nodeOptions = computed(() => (facets.value?.node ?? []).map((f) => ({ value: f.value, label: f.label, count: f.count })))

function toggleGame(prefab: string) {
  const current = query.filters.value.prefab ?? []
  query.setFilter('prefab', current.includes(prefab) ? current.filter((p) => p !== prefab) : [...current, prefab])
}
function toggleFlag(key: 'upgrade' | 'suspended') {
  query.setFilter(key, query.filters.value[key]?.[0] === 'true' ? [] : ['true'])
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('servers.title')">
      <CreateServerDialog />
    </PageHeader>
    <div class="grid min-w-0 gap-4 p-6">
      <div v-if="isPending && !page" class="grid gap-2">
        <Skeleton v-for="index in 3" :key="index" class="h-12" />
      </div>
      <Empty v-else-if="total === 0 && !query.active.value" :icon="ServerIcon" :title="t('servers.empty.title')">
        {{ t('servers.empty.help') }}
      </Empty>
      <template v-else>
        <!-- Filtres rapides par jeu. -->
        <div v-if="games.length > 1" class="flex flex-wrap gap-2" role="group" :aria-label="t('servers.list.games')">
          <Button
            v-for="game in games"
            :key="game.value"
            size="sm"
            :variant="query.filters.value.prefab?.includes(game.value) ? 'default' : 'outline'"
            class="h-8 gap-2"
            :aria-pressed="query.filters.value.prefab?.includes(game.value) ?? false"
            @click="toggleGame(game.value)"
          >
            <!-- Libellé de la facette prefab : sa dernière version, pour l'icône. -->
            <PrefabIcon :prefab="{ id: game.value, version: game.label }" class="size-5 rounded text-[10px]" />
            <span class="font-mono text-xs">{{ game.value }}</span>
            <span class="text-xs opacity-70">{{ game.count }}</span>
          </Button>
        </div>

        <div class="flex flex-wrap items-center gap-2">
          <DataTableSearch v-model="query.q.value" :placeholder="t('servers.list.search')" />
          <DataTableFacetFilter
            :model-value="query.filters.value.state ?? []"
            :title="t('servers.fields.state')"
            :options="stateOptions"
            @update:model-value="(values) => query.setFilter('state', values)"
          />
          <DataTableFacetFilter
            :model-value="query.filters.value.node ?? []"
            :title="t('servers.fields.node')"
            :options="nodeOptions"
            @update:model-value="(values) => query.setFilter('node', values)"
          />
          <Button
            size="sm"
            class="h-8 border-dashed"
            :variant="query.filters.value.upgrade?.[0] === 'true' ? 'default' : 'outline'"
            :aria-pressed="query.filters.value.upgrade?.[0] === 'true'"
            @click="toggleFlag('upgrade')"
          >
            <ArrowUpCircle />{{ t('servers.list.upgradeAvailable', { count: facets?.upgrade ?? 0 }) }}
          </Button>
          <Button
            v-if="(facets?.suspended ?? 0) > 0 || query.filters.value.suspended?.length"
            size="sm"
            class="h-8 border-dashed"
            :variant="query.filters.value.suspended?.[0] === 'true' ? 'default' : 'outline'"
            :aria-pressed="query.filters.value.suspended?.[0] === 'true'"
            @click="toggleFlag('suspended')"
          >
            {{ t('servers.list.suspended', { count: facets?.suspended ?? 0 }) }}
          </Button>
          <Button v-if="query.active.value" variant="ghost" size="sm" class="h-8" @click="query.reset()">
            {{ t('dataTable.reset') }}<X />
          </Button>
          <DataTableViewOptions :table="table" />
        </div>

        <DataTableSelectionBar
          :selected="selectedIds.length"
          :page-selected="pageSelected"
          :total="total"
          :all-matching="allMatching"
          @select-all="allMatching = true"
          @clear="clear"
        >
          <ServerBulkActions :count="targetCount" :resolve="resolve" :names="names" @done="clear" />
        </DataTableSelectionBar>

        <DataTable :table="table" :row-to="(server) => `/servers/${server.id}`" :loading="isPending" />
        <DataTablePagination :table="table" :query="query" :total="total" />
      </template>
    </div>
  </div>
</template>
