<script setup lang="ts">
// Nœuds sur la carte (ADR-0021) : un point par nœud placé (ses coordonnées, sinon le centre de
// sa région), coloré par état, avec une bulle (région, lieu, serveurs, lien vers le nœud). Les
// centres des régions apparaissent en cercles discrets.
import { computed, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Region } from '@/features/regions/regions.api'
import { Badge } from '@/shared/ui/badge'
import { MapMarker, MapView } from '@/shared/ui/map'

import type { Node } from '../nodes.api'
import { nodeBadge, nodeDot } from '../status'

const props = defineProps<{ nodes: Node[]; regions?: Region[]; servers?: Record<string, number>; class?: string }>()
const { t } = useI18n()
const view = useTemplateRef<InstanceType<typeof MapView>>('view')

const placed = computed(() => props.nodes.filter((node) => node.map_position))

// Cadrage sur les nœuds placés à la première donnée reçue, puis l'admin garde la main.
let framed = false
watch(
  [placed, view],
  ([nodes, map]) => {
    if (framed || !map || nodes.length === 0) return
    map.fit(nodes.map((node) => node.map_position!))
    framed = true
  },
  { immediate: true },
)
</script>

<template>
  <MapView ref="view" :class="props.class">
    <MapMarker v-for="region in regions ?? []" :key="region.id" :position="region" decorative>
      <span class="block size-10 rounded-full border border-primary/40 bg-primary/10" :title="region.name" />
    </MapMarker>
    <MapMarker v-for="node in placed" :key="node.id" :position="node.map_position!">
      <span
        class="relative block size-3.5 rounded-full border-2 border-white shadow-[0_1px_3px_oklch(0_0_0/40%)]"
        :class="nodeDot[node.status]"
        :aria-label="node.display_name ?? node.name"
      >
        <span v-if="node.status === 'online'" class="absolute inset-0 animate-ping rounded-full bg-success/60 motion-reduce:hidden" />
      </span>
      <template #popup>
        <p class="font-bold">{{ node.display_name ?? node.name }}</p>
        <Badge :variant="nodeBadge[node.status]" class="h-5">{{ t(`nodes.status.${node.status}`) }}</Badge>
        <p v-if="node.region || node.location" class="text-xs text-muted-foreground">
          {{ [node.region?.name, node.location].filter(Boolean).join(' · ') }}
        </p>
        <p v-if="servers" class="text-xs text-muted-foreground">{{ t('overview.servers', servers[node.id] ?? 0) }}</p>
        <RouterLink :to="`/servers/nodes/${node.id}`" class="text-[13px] font-semibold text-primary hover:underline">
          {{ t('worldMap.openNode') }}
        </RouterLink>
      </template>
    </MapMarker>
  </MapView>
</template>
