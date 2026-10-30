<script setup lang="ts">
// Espace occupé (volumes et sauvegardes) par rapport au quota, d'après la dernière mesure du nœud.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { formatBytes } from '@/shared/lib/format'

import type { Server } from '../servers.api'
import { diskQuota, isOverQuota } from '../state'

const props = defineProps<{ server: Server }>()
const { t, locale } = useI18n()

const quota = computed(() => diskQuota(props.server))
const used = computed(() => props.server.disk_used_bytes)
const ratio = computed(() => (quota.value && used.value != null ? Math.min(1, used.value / quota.value) : 0))
const tone = computed(() => (isOverQuota(props.server) ? 'bg-destructive' : ratio.value >= 0.9 ? 'bg-warning' : 'bg-primary'))
</script>

<template>
  <div class="grid gap-1.5">
    <p class="text-sm tabular-nums">
      <template v-if="used == null">{{ t('servers.disk.unmeasured') }}</template>
      <template v-else-if="quota">{{ t('servers.disk.usage', { used: formatBytes(used, locale), quota: formatBytes(quota, locale) }) }}</template>
      <template v-else>{{ t('servers.disk.unlimited', { used: formatBytes(used, locale) }) }}</template>
    </p>
    <div
      v-if="quota"
      class="h-1.5 overflow-hidden rounded-full bg-muted"
      role="meter"
      :aria-label="t('servers.disk.label')"
      :aria-valuenow="Math.round(ratio * 100)"
      aria-valuemin="0"
      aria-valuemax="100"
    >
      <div class="h-full rounded-full transition-all" :class="tone" :style="{ width: `${ratio * 100}%` }" />
    </div>
  </div>
</template>
