<script setup lang="ts">
// Carte du monde des nœuds (ADR-0021) : pleine page, filtrable par région, légende des états
// avec leur nombre.
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import NodesMap from '@/features/nodes/components/NodesMap.vue'
import { useNodes } from '@/features/nodes/nodes.queries'
import { NODE_STATUSES, nodeDot } from '@/features/nodes/status'
import { useRegions } from '@/features/regions/regions.queries'
import { useFleetMetrics } from '@/features/servers/composables/useFleetMetrics'
import { useServers } from '@/features/servers/servers.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

const ALL = '*'
const NONE = '-'

const { t } = useI18n()
const { data: nodes } = useNodes()
const { data: regions } = useRegions()
const { data: servers } = useServers()
const metrics = useFleetMetrics(servers, nodes)
const region = ref(ALL)

const shown = computed(() =>
  (nodes.value ?? []).filter((node) => region.value === ALL || (region.value === NONE ? !node.region : node.region?.id === region.value)),
)
const shownRegions = computed(() => (regions.value ?? []).filter((r) => region.value === ALL || r.id === region.value))
const regionLabel = computed(() =>
  region.value === ALL ? t('worldMap.allRegions') : region.value === NONE ? t('worldMap.noRegion') : regions.value?.find((r) => r.id === region.value)?.name,
)
const counts = computed(() => Object.fromEntries(NODE_STATUSES.map((status) => [status, shown.value.filter((node) => node.status === status).length])))
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <PageHeader :title="t('worldMap.title')">
      <Select v-model="region">
        <SelectTrigger class="w-56" :aria-label="t('worldMap.region')">
          <SelectValue>{{ regionLabel }}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem :value="ALL">{{ t('worldMap.allRegions') }}</SelectItem>
          <SelectItem v-for="r in regions ?? []" :key="r.id" :value="r.id">{{ r.name }}</SelectItem>
          <SelectItem :value="NONE">{{ t('worldMap.noRegion') }}</SelectItem>
        </SelectContent>
      </Select>
    </PageHeader>
    <div class="flex min-h-0 flex-1 flex-col gap-3 p-6">
      <ul :aria-label="t('worldMap.legend')" class="flex flex-wrap gap-4 text-[13px] text-muted-foreground">
        <li v-for="status in NODE_STATUSES" :key="status" class="inline-flex items-center gap-1.5" :data-status="status">
          <span class="size-2.5 rounded-full" :class="nodeDot[status]" aria-hidden="true" />
          {{ t(`nodes.status.${status}`) }}
          <span class="font-semibold text-foreground tabular-nums">{{ counts[status] }}</span>
        </li>
      </ul>
      <NodesMap :key="region" class="min-h-96 flex-1" :nodes="shown" :regions="shownRegions" :servers="metrics.perNode" />
    </div>
  </div>
</template>
