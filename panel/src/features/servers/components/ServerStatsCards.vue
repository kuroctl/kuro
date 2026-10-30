<script setup lang="ts">
// CPU, mémoire et réseau en direct, avec la courbe des 5 dernières minutes.
import { Cpu, MemoryStick, Network } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { formatBytes, formatPercent } from '@/shared/lib/format'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'

import { STATS_WINDOW, type StatsSample } from '../composables/useServerConsole'
import type { Server } from '../servers.api'
import StatsSparkline from './StatsSparkline.vue'

const props = defineProps<{ server: Server; samples: StatsSample[] }>()

const MEASURED: Server['state'][] = ['starting', 'running', 'stopping']

const { t, locale } = useI18n()
// Serveur arrêté : le nœud n'envoie plus rien, la dernière mesure ne vaut plus.
const latest = computed(() => (MEASURED.includes(props.server.state) ? props.samples.at(-1) : undefined))
const bytes = (value: number) => formatBytes(value, locale.value)
</script>

<template>
  <div class="grid gap-4 md:grid-cols-3">
    <Card class="gap-2 py-4">
      <CardHeader class="px-4">
        <CardTitle class="flex items-center gap-1.5 text-xs font-normal text-muted-foreground"><Cpu class="size-4" />{{ t('servers.console.stats.cpu') }}</CardTitle>
      </CardHeader>
      <CardContent class="grid gap-2 px-4">
        <p v-if="latest" class="text-[22px] leading-tight font-extrabold tracking-tight tabular-nums">
          {{ formatPercent(latest.cpu_millis / 10, locale) }}
          <span class="text-[13px] font-semibold text-muted-foreground">{{ t('servers.console.stats.of', { max: formatPercent(server.cpu_percent, locale) }) }}</span>
        </p>
        <p v-else class="text-sm text-muted-foreground">{{ t('servers.console.stats.noData') }}</p>
        <StatsSparkline :series="[samples.map((s) => s.cpu_millis)]" :window="STATS_WINDOW" :max="server.cpu_percent * 10" />
      </CardContent>
    </Card>

    <Card class="gap-2 py-4">
      <CardHeader class="px-4">
        <CardTitle class="flex items-center gap-1.5 text-xs font-normal text-muted-foreground"><MemoryStick class="size-4" />{{ t('servers.console.stats.memory') }}</CardTitle>
      </CardHeader>
      <CardContent class="grid gap-2 px-4">
        <p v-if="latest" class="text-[22px] leading-tight font-extrabold tracking-tight tabular-nums">
          {{ bytes(latest.memory_bytes) }}
          <span class="text-[13px] font-semibold text-muted-foreground">{{ t('servers.console.stats.of', { max: bytes(latest.memory_limit_bytes) }) }}</span>
        </p>
        <p v-else class="text-sm text-muted-foreground">{{ t('servers.console.stats.noData') }}</p>
        <StatsSparkline :series="[samples.map((s) => s.memory_bytes)]" :window="STATS_WINDOW" :max="latest?.memory_limit_bytes" />
      </CardContent>
    </Card>

    <Card class="gap-2 py-4">
      <CardHeader class="px-4">
        <CardTitle class="flex items-center gap-1.5 text-xs font-normal text-muted-foreground"><Network class="size-4" />{{ t('servers.console.stats.network') }}</CardTitle>
      </CardHeader>
      <CardContent class="grid gap-2 px-4">
        <p v-if="latest" class="text-[15px] font-bold tabular-nums">
          {{ t('servers.console.stats.rates', { rx: bytes(latest.rx_per_sec), tx: bytes(latest.tx_per_sec) }) }}
        </p>
        <p v-else class="text-sm text-muted-foreground">{{ t('servers.console.stats.noData') }}</p>
        <StatsSparkline :series="[samples.map((s) => s.rx_per_sec), samples.map((s) => s.tx_per_sec)]" :window="STATS_WINDOW" />
      </CardContent>
    </Card>
  </div>
</template>
