<script setup lang="ts">
// Régions : emplacements physiques des nœuds, leur centre sur la carte et l'état de leurs
// nœuds. Seule une région vide peut être supprimée (l'API reste seule juge). Liste en mode
// local (ADR-0025) : recherche ici, tri et pagination par la table, état dans l'URL.
import { Globe, MoreHorizontal, Pencil, Plus, Trash2 } from '@lucide/vue'
import { createColumnHelper } from '@tanstack/vue-table'
import { computed, h, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import RegionDialog from '@/features/regions/components/RegionDialog.vue'
import { type Region, useRegionsApi } from '@/features/regions/regions.api'
import { useInvalidateRegions, useRegions } from '@/features/regions/regions.queries'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Button } from '@/shared/ui/button'
import {
  DataTable,
  type DataTableFeatures,
  DataTablePagination,
  DataTableSearch,
  sortable,
  useDataTable,
  useTableQuery,
} from '@/shared/ui/data-table'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/shared/ui/dropdown-menu'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'

const { t, n } = useI18n()
const { can } = usePermissions()
const { data: regions, isPending } = useRegions()
const api = useRegionsApi()
const invalidate = useInvalidateRegions()
const { handleError } = useApiError()

const editing = ref<Region>()
const dialog = ref(false)
const deleting = ref<Region>()
const pending = ref(false)

function openDialog(region?: Region) {
  editing.value = region
  dialog.value = true
}

async function remove() {
  const region = deleting.value
  if (!region) return
  pending.value = true
  try {
    await api.remove(region.id)
    toast.success(t('regions.delete.done', { name: region.name }))
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
    deleting.value = undefined
  }
}

const coordinate = (value: number) => n(value, { maximumFractionDigits: 3 })

const query = useTableQuery({ sort: 'name' })
const all = computed(() => regions.value ?? [])
/** Recherche sur le nom et l'identifiant. */
const filtered = computed(() => {
  const needle = query.q.value.toLowerCase()
  return needle ? all.value.filter((r) => r.name.toLowerCase().includes(needle) || r.slug.includes(needle)) : all.value
})

const column = createColumnHelper<DataTableFeatures, Region>()

const columns = column.columns([
  column.accessor('name', {
    id: 'name',
    meta: { title: t('regions.fields.name'), class: 'font-semibold' },
    header: sortable(t('regions.fields.name')),
  }),
  column.accessor('slug', {
    id: 'slug',
    meta: { title: t('regions.fields.slug'), class: 'font-mono text-xs' },
    header: sortable(t('regions.fields.slug')),
  }),
  column.accessor((region) => region.nodes.total, {
    id: 'nodes',
    meta: { title: t('regions.fields.nodes') },
    header: sortable(t('regions.fields.nodes')),
    cell: ({ row }) => t('regions.nodes', { online: row.original.nodes.online, total: row.original.nodes.total }),
  }),
  column.display({
    id: 'center',
    meta: { title: t('regions.fields.center'), class: 'font-mono text-xs text-muted-foreground' },
    header: () => t('regions.fields.center'),
    cell: ({ row }) => `${coordinate(row.original.latitude)}, ${coordinate(row.original.longitude)}`,
  }),
  // Colonne toujours présente (les permissions arrivent après la création de la table) : vide
  // sans le droit de gérer les régions.
  column.display({
    id: 'actions',
    meta: { class: 'w-12 text-right' },
    cell: ({ row }) => (can('nodes.regions.manage') ? h(RegionActions, { region: row.original }) : null),
  }),
])

/** Menu d'actions d'une ligne : modifier, supprimer. */
function RegionActions({ region }: { region: Region }) {
  return h(DropdownMenu, null, () => [
    h(DropdownMenuTrigger, { asChild: true }, () =>
      h(Button, { variant: 'ghost', size: 'icon-sm', 'aria-label': t('regions.actions', { name: region.name }) }, () => h(MoreHorizontal)),
    ),
    h(DropdownMenuContent, { align: 'end' }, () => [
      h(DropdownMenuItem, { onSelect: () => openDialog(region) }, () => [h(Pencil), t('regions.edit')]),
      h(DropdownMenuItem, { variant: 'destructive', onSelect: () => (deleting.value = region) }, () => [h(Trash2), t('regions.delete.action')]),
    ]),
  ])
}
RegionActions.props = ['region']

const { table } = useDataTable({ data: filtered, columns, getRowId: (region) => region.id, query })
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('regions.title')">
      <Button v-if="can('nodes.regions.manage')" @click="openDialog()"><Plus />{{ t('regions.add') }}</Button>
    </PageHeader>
    <div class="grid min-w-0 gap-4 p-6">
      <p class="max-w-3xl text-sm text-muted-foreground">{{ t('regions.help') }}</p>
      <p v-if="!can('nodes.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t('regions.forbidden') }}</p>
      <div v-else-if="isPending" class="grid gap-2"><Skeleton v-for="index in 3" :key="index" class="h-12" /></div>
      <Empty v-else-if="!regions?.length" :icon="Globe">{{ t('regions.empty') }}</Empty>
      <template v-else>
        <DataTableSearch v-model="query.q.value" :placeholder="t('regions.search')" />
        <DataTable :table="table" />
        <DataTablePagination :table="table" :query="query" :total="filtered.length" />
      </template>
    </div>

    <RegionDialog v-model:open="dialog" :region="editing" />

    <AlertDialog :open="!!deleting" @update:open="(open) => !open && (deleting = undefined)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('regions.delete.title', { name: deleting?.name ?? '' }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('regions.delete.help') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="remove">{{ t('regions.delete.confirm') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
