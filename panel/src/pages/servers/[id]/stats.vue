<script setup lang="ts">
// Historique des stats d'un serveur (ADR-0043) : CPU, mémoire et réseau sur 24 h (à la minute),
// 7 j ou 90 j (à l'heure), envoyés par le nœud une fois par minute.
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import HistoryChart from '@/features/servers/components/HistoryChart.vue'
import { type MetricsRange, useServerMetrics } from '@/features/servers/servers.queries'
import { useServer } from '@/features/servers/servers.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { formatBytes, formatCores, formatPercent } from '@/shared/lib/format'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Skeleton } from '@/shared/ui/skeleton'

definePage({ meta: { subnav: 'server' } })

const RANGES: MetricsRange[] = ['24h', '7d', '90d']

const { t, locale } = useI18n()
const route = useRoute('/servers/[id]/stats')
const id = computed(() => route.params.id)
const { data: server } = useServer(id)
const range = ref<MetricsRange>('24h')
const { data: metrics, isPending } = useServerMetrics(id, range)

const points = computed(() => metrics.value?.points ?? [])
const cpu = computed(() => points.value.map((p) => ({ at: p.at, values: [p.cpu_millis_avg / 10, p.cpu_millis_max / 10] })))
const memory = computed(() => points.value.map((p) => ({ at: p.at, values: [p.memory_bytes_avg, p.memory_bytes_max] })))
const network = computed(() => points.value.map((p) => ({ at: p.at, values: [p.network_rx_bytes, p.network_tx_bytes] })))
// La courbe reste en % d'un cœur (`cpu_millis / 10`) ; seul le titre parle en cœurs.
const memoryMax = computed(() => (server.value ? server.value.memory_mb * 1024 * 1024 : undefined))
const percent = (value: number) => formatPercent(value, locale.value)
const bytes = (value: number) => formatBytes(value, locale.value)
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('servers.stats.title')">
      <Select v-model="range">
        <SelectTrigger class="w-36" :aria-label="t('servers.stats.range')"><SelectValue>{{ t(`servers.stats.ranges.${range}`) }}</SelectValue></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="option in RANGES" :key="option" :value="option">{{ t(`servers.stats.ranges.${option}`) }}</SelectItem>
        </SelectContent>
      </Select>
    </PageHeader>
    <div class="grid gap-4 p-6">
      <p class="max-w-3xl text-sm text-muted-foreground">{{ t('servers.stats.help') }}</p>
      <div v-if="isPending || !server" class="grid gap-4"><Skeleton class="h-48" /><Skeleton class="h-48" /></div>
      <p v-else-if="!points.length" class="text-sm text-muted-foreground" data-empty>{{ t('servers.stats.empty') }}</p>
      <template v-else>
        <Card class="gap-2 py-4">
          <CardHeader class="px-4"><CardTitle class="text-sm">{{ t('servers.stats.cpu', { max: formatCores(server.cpu_percent, locale) }) }}</CardTitle></CardHeader>
          <CardContent class="px-4">
            <HistoryChart :points="cpu" :labels="[t('servers.stats.average'), t('servers.stats.peak')]" :format="percent" :max="server.cpu_percent" :range="range" />
          </CardContent>
        </Card>
        <Card class="gap-2 py-4">
          <CardHeader class="px-4"><CardTitle class="text-sm">{{ t('servers.stats.memory', { max: bytes(memoryMax ?? 0) }) }}</CardTitle></CardHeader>
          <CardContent class="px-4">
            <HistoryChart :points="memory" :labels="[t('servers.stats.average'), t('servers.stats.peak')]" :format="bytes" :max="memoryMax" :range="range" />
          </CardContent>
        </Card>
        <Card class="gap-2 py-4">
          <CardHeader class="px-4"><CardTitle class="text-sm">{{ t('servers.stats.network', { step: t(`servers.stats.steps.${metrics?.step_secs === 60 ? 'minute' : 'hour'}`) }) }}</CardTitle></CardHeader>
          <CardContent class="px-4">
            <HistoryChart :points="network" :labels="[t('servers.stats.received'), t('servers.stats.sent')]" :format="bytes" :range="range" />
          </CardContent>
        </Card>
      </template>
    </div>
  </div>
</template>
