<script setup lang="ts">
// Journal d'audit (ADR-0008, ADR-0019, ADR-0025) : liste filtrée par l'API et parcourue par curseur
// (`before_seq`, « Événements plus anciens »), rendue par la table partagée (colonnes affichables,
// sans tri : l'API ne trie pas). Filtres et événement ouvert tiennent dans l'URL. Vérification de
// la chaîne de hash, export CSV. Chaque consultation est elle-même journalisée par l'API.
import { Download, ShieldCheck } from '@lucide/vue'
import { createColumnHelper } from '@tanstack/vue-table'
import { computed, h, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { type LocationQuery, useRoute, useRouter } from 'vue-router'

import { usePermissions } from '@/features/account/account.queries'
import { type AuditEntry, type AuditFilters, type AuditVerify, useAuditApi } from '@/features/audit/audit.api'
import { useAuditLog } from '@/features/audit/audit.queries'
import AuditEntryDialog from '@/features/audit/components/AuditEntryDialog.vue'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { DataTable, type DataTableFeatures, DataTableViewOptions, plain, useDataTable, useTableQuery } from '@/shared/ui/data-table'
import { DatePicker } from '@/shared/ui/date-picker'
import { Label } from '@/shared/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Skeleton } from '@/shared/ui/skeleton'

const ALL = 'all'
const FAMILIES = ['auth.', 'iam.', 'access.', 'server.', 'node.', 'region.', 'allocation.', 'prefab.', 'dns.', 'content_source.', 'pki.', 'system.', 'audit.']
const ACTOR_KINDS = ['admin', 'system', 'node', 'cli', 'anonymous']
const OUTCOMES = ['success', 'denied', 'failure']
const FILTERS = ['action', 'actor_kind', 'outcome', 'from', 'to'] as const
type FilterKey = (typeof FILTERS)[number]

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const { can, profile } = usePermissions()
const api = useAuditApi()
const { handleError, message } = useApiError()

// L'ordre est celui de l'API (du plus récent au plus ancien) ; le tri n'est déclaré que parce que
// `useTableQuery` l'exige, aucune colonne n'est triable.
const query = useTableQuery({ sort: 'date', order: 'desc', filters: FILTERS })

/** Un filtre du formulaire, lu et écrit dans l'URL ; `fallback` quand il est absent. */
function field(key: FilterKey, fallback: string) {
  return computed({
    get: () => query.filters.value[key]?.[0] ?? fallback,
    set: (value: string) => query.setFilter(key, value && value !== ALL ? [value] : []),
  })
}
const form = {
  action: field('action', ALL),
  actor_kind: field('actor_kind', ALL),
  outcome: field('outcome', ALL),
  from: field('from', ''),
  to: field('to', ''),
}

/** Filtres envoyés : période en jours entiers, `to` inclus. */
const filters = computed<AuditFilters>(() => {
  const day = (value: string, offset = 0) => {
    if (!value) return undefined
    const date = new Date(`${value}T00:00:00`)
    date.setDate(date.getDate() + offset)
    return date.toISOString()
  }
  const pick = (value: string) => (value === ALL ? undefined : value)
  return {
    action: pick(form.action.value),
    actor_kind: pick(form.actor_kind.value),
    outcome: pick(form.outcome.value),
    from: day(form.from.value),
    to: day(form.to.value, 1),
  }
})

const log = useAuditLog(filters)
const entries = computed(() => log.data.value?.pages.flatMap((page) => page.items) ?? [])
const verifying = ref(false)
const verified = ref<AuditVerify | null>(null)
const exporting = ref(false)
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'short', timeStyle: 'medium' }))

/** Événement ouvert (`?entry=<seq>`), parmi ceux déjà chargés. */
const selected = computed(() => entries.value.find((entry) => String(entry.seq) === route.query.entry) ?? null)
const entryLink = (entry: AuditEntry) => router.resolve({ query: { ...route.query, entry: String(entry.seq) } }).fullPath
function closeEntry() {
  const rest: LocationQuery = { ...route.query }
  delete rest.entry
  void router.replace({ query: rest })
}

function actor(entry: AuditEntry): string {
  if (entry.actor_id && entry.actor_id === profile.value?.user.id) return t('audit.you')
  return t(`audit.actorKind.${entry.actor_kind}`)
}

const outcomeVariant = (outcome: string) => (outcome === 'success' ? 'secondary' : outcome === 'denied' ? 'outline' : 'destructive')

const column = createColumnHelper<DataTableFeatures, AuditEntry>()
/** En-tête non triable, au style des en-têtes triables. */

const columns = column.columns([
  column.accessor('occurred_at', {
    id: 'date',
    enableSorting: false,
    enableHiding: false,
    meta: { title: t('audit.columns.date'), class: 'w-44 text-muted-foreground tabular-nums' },
    header: plain(t('audit.columns.date')),
    cell: ({ row }) => dates.value.format(new Date(row.original.occurred_at)),
  }),
  column.accessor('seq', {
    id: 'seq',
    enableSorting: false,
    meta: { title: t('audit.detail.seq'), class: 'w-24 font-mono text-xs text-muted-foreground tabular-nums' },
    header: plain(t('audit.detail.seq')),
  }),
  column.accessor('actor_kind', {
    id: 'actor',
    enableSorting: false,
    meta: { title: t('audit.columns.actor'), class: 'w-36' },
    header: plain(t('audit.columns.actor')),
    cell: ({ row }) => actor(row.original),
  }),
  column.accessor('action', {
    id: 'action',
    enableSorting: false,
    enableHiding: false,
    meta: { title: t('audit.columns.action'), class: 'font-mono text-xs' },
    header: plain(t('audit.columns.action')),
  }),
  column.accessor('outcome', {
    id: 'outcome',
    enableSorting: false,
    meta: { title: t('audit.columns.outcome'), class: 'w-28' },
    header: plain(t('audit.columns.outcome')),
    cell: ({ row }) => h(Badge, { variant: outcomeVariant(row.original.outcome) }, () => t(`audit.outcome.${row.original.outcome}`)),
  }),
  column.display({
    id: 'target',
    meta: { title: t('audit.columns.target'), class: 'max-w-80 truncate font-mono text-xs text-muted-foreground' },
    header: plain(t('audit.columns.target')),
    cell: ({ row }) => [row.original.target_kind, row.original.target_id].filter(Boolean).join(' · '),
  }),
  column.accessor((entry) => entry.ip ?? '', {
    id: 'ip',
    enableSorting: false,
    meta: { title: t('audit.detail.ip'), class: 'w-36 font-mono text-xs text-muted-foreground' },
    header: plain(t('audit.detail.ip')),
  }),
])

// Pagination par curseur : la table reçoit tout ce qui est chargé, en mode API (`rowCount`) pour
// qu'elle ne pagine ni ne trie elle-même ; « Événements plus anciens » ajoute la page suivante.
const { table } = useDataTable({
  data: entries,
  columns,
  getRowId: (entry) => String(entry.seq),
  query,
  rowCount: () => entries.value.length,
  storageKey: 'audit',
  hidden: ['seq', 'ip'],
})

async function verify() {
  verifying.value = true
  try {
    verified.value = await api.verify()
  } catch (error) {
    handleError(error)
  } finally {
    verifying.value = false
  }
}

async function exportCsv() {
  exporting.value = true
  try {
    const { blob, disposition } = await api.exportCsv(filters.value)
    const name = /filename="([^"]+)"/.exec(disposition)?.[1] ?? 'kuro-audit.csv'
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = name
    link.click()
    URL.revokeObjectURL(url)
  } catch (error) {
    handleError(error)
  } finally {
    exporting.value = false
  }
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <PageHeader :title="t('subnav.settings.audit')">
      <template v-if="can('audit.read')">
        <Button size="sm" variant="outline" :disabled="verifying" @click="verify"><ShieldCheck />{{ t('audit.verify.action') }}</Button>
        <Button size="sm" variant="outline" :disabled="exporting" @click="exportCsv"><Download />{{ t('audit.export') }}</Button>
      </template>
    </PageHeader>
    <div class="grid min-w-0 gap-4 p-6">
      <p v-if="!can('audit.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('audit.forbidden') }}
      </p>
      <template v-else>
        <p
          v-if="verified"
          role="status"
          class="rounded-md border p-3 text-sm"
          :class="verified.intact ? 'border-success/35 bg-success/8' : 'border-destructive/35 bg-destructive/8'"
        >
          {{ verified.intact ? t('audit.verify.intact', { count: verified.checked }) : t('audit.verify.broken', { seq: verified.broken_at }) }}
        </p>

        <div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-[repeat(5,minmax(0,1fr))_auto_auto] lg:items-end">
          <div class="grid gap-1.5">
            <Label>{{ t('audit.filters.action') }}</Label>
            <Select v-model="form.action.value">
              <SelectTrigger class="w-full" :aria-label="t('audit.filters.action')">
                <SelectValue>{{ form.action.value === ALL ? t('audit.filters.all') : t(`audit.family.${form.action.value.slice(0, -1)}`) }}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem :value="ALL">{{ t('audit.filters.all') }}</SelectItem>
                <SelectItem v-for="family in FAMILIES" :key="family" :value="family">{{ t(`audit.family.${family.slice(0, -1)}`) }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div class="grid gap-1.5">
            <Label>{{ t('audit.filters.actor') }}</Label>
            <Select v-model="form.actor_kind.value">
              <SelectTrigger class="w-full" :aria-label="t('audit.filters.actor')">
                <SelectValue>{{ form.actor_kind.value === ALL ? t('audit.filters.all') : t(`audit.actorKind.${form.actor_kind.value}`) }}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem :value="ALL">{{ t('audit.filters.all') }}</SelectItem>
                <SelectItem v-for="kind in ACTOR_KINDS" :key="kind" :value="kind">{{ t(`audit.actorKind.${kind}`) }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div class="grid gap-1.5">
            <Label>{{ t('audit.filters.outcome') }}</Label>
            <Select v-model="form.outcome.value">
              <SelectTrigger class="w-full" :aria-label="t('audit.filters.outcome')">
                <SelectValue>{{ form.outcome.value === ALL ? t('audit.filters.all') : t(`audit.outcome.${form.outcome.value}`) }}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem :value="ALL">{{ t('audit.filters.all') }}</SelectItem>
                <SelectItem v-for="outcome in OUTCOMES" :key="outcome" :value="outcome">{{ t(`audit.outcome.${outcome}`) }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div class="grid gap-1.5">
            <Label for="audit-from">{{ t('audit.filters.from') }}</Label>
            <DatePicker id="audit-from" v-model="form.from.value" />
          </div>
          <div class="grid gap-1.5">
            <Label for="audit-to">{{ t('audit.filters.to') }}</Label>
            <DatePicker id="audit-to" v-model="form.to.value" />
          </div>
          <Button variant="ghost" size="sm" @click="query.reset()">{{ t('audit.filters.reset') }}</Button>
          <DataTableViewOptions :table="table" />
        </div>

        <p v-if="log.error.value" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(log.error.value) }}</p>
        <div v-else-if="log.isPending.value" class="grid gap-2"><Skeleton v-for="index in 6" :key="index" class="h-9" /></div>
        <p v-else-if="entries.length === 0" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t('audit.empty') }}</p>
        <template v-else>
          <DataTable :table="table" :row-to="entryLink" />
          <div v-if="log.hasNextPage.value" class="flex justify-center">
            <Button variant="outline" size="sm" :disabled="log.isFetchingNextPage.value" @click="log.fetchNextPage()">{{ t('audit.more') }}</Button>
          </div>
        </template>
      </template>
    </div>
    <AuditEntryDialog :entry="selected" @close="closeEntry" />
  </div>
</template>
