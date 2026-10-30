<script setup lang="ts">
// Vue d'ensemble de l'infrastructure, première page de la section : la carte des nœuds reste en
// haut pendant que les indicateurs et les derniers serveurs défilent par-dessus.
import { ArrowRight, HardDrive, MemoryStick, Network, Plus, Server, TriangleAlert } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import NodesMap from '@/features/nodes/components/NodesMap.vue'
import { useNodes } from '@/features/nodes/nodes.queries'
import PrefabIcon from '@/features/prefabs/components/PrefabIcon.vue'
import { useRegions } from '@/features/regions/regions.queries'
import ServerStateBadge from '@/features/servers/components/ServerStateBadge.vue'
import { useFleetMetrics } from '@/features/servers/composables/useFleetMetrics'
import { useServers } from '@/features/servers/servers.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { formatBytes } from '@/shared/lib/format'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'

const LATEST = 5

const { t, locale } = useI18n()
const { can } = usePermissions()
const { data: servers, isPending } = useServers()
const { data: nodes } = useNodes()
const { data: regions } = useRegions()
const metrics = useFleetMetrics(servers, nodes)
const bytes = (value: number) => formatBytes(value, locale.value)

const latest = computed(() =>
  [...(servers.value ?? [])].sort((a, b) => b.created_at.localeCompare(a.created_at)).slice(0, LATEST),
)

interface Metric {
  key: string
  icon: typeof Server
  label: string
  value: string
  detail: string
  /** Remplissage de la jauge, entre 0 et 1 ; absent : pas de jauge. */
  ratio?: number
  tone?: string
}

const cards = computed<Metric[]>(() => {
  const m = metrics.value
  const ratio = (part: number, whole: number | null) => (whole ? Math.min(1, part / whole) : undefined)
  const tone = (value?: number) => (value === undefined ? undefined : value >= 0.9 ? 'bg-destructive' : value >= 0.75 ? 'bg-warning' : 'bg-primary')
  const memory = ratio(m.memoryAllocated, m.memoryCapacity)
  const disk = ratio(m.diskUsed, m.diskQuota)
  return [
    { key: 'servers', icon: Server, label: t('overview.metrics.servers'), value: String(m.servers), detail: t('overview.metrics.running', { count: m.running }) },
    {
      key: 'attention',
      icon: TriangleAlert,
      label: t('overview.metrics.attention'),
      value: String(m.attention),
      detail: t('overview.metrics.attentionHelp'),
      tone: m.attention > 0 ? 'text-warning' : undefined,
    },
    {
      key: 'nodes',
      icon: Network,
      label: t('overview.metrics.nodes'),
      value: `${m.nodesOnline} / ${m.nodes}`,
      detail: t('overview.metrics.pendingNodes', { count: m.nodesPending }),
      ratio: ratio(m.nodesOnline, m.nodes),
    },
    {
      key: 'memory',
      icon: MemoryStick,
      label: t('overview.metrics.memory'),
      value: bytes(m.memoryAllocated),
      detail: m.memoryCapacity ? t('overview.metrics.memoryOf', { total: bytes(m.memoryCapacity) }) : t('overview.metrics.memoryUnknown'),
      ratio: memory,
      tone: tone(memory),
    },
    {
      key: 'disk',
      icon: HardDrive,
      label: t('overview.metrics.disk'),
      value: bytes(m.diskUsed),
      detail: m.diskQuota ? t('overview.metrics.diskOf', { total: bytes(m.diskQuota) }) : t('overview.metrics.diskUnlimited'),
      ratio: disk,
      tone: tone(disk),
    },
  ]
})
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('overview.title')">
      <Button v-if="can('servers.create')" size="sm" as-child>
        <RouterLink to="/servers/new"><Plus />{{ t('servers.create.action') }}</RouterLink>
      </Button>
    </PageHeader>

      <!-- La carte reste en haut ; le contenu glisse par-dessus en défilant. -->
      <NodesMap
        v-if="can('nodes.read')"
        class="sticky top-6 z-0 h-[42vh] min-h-72 border-none rounded-none [mask-image:linear-gradient(to_bottom,transparent_0%,black_20%,black_80%,transparent_100%)]"
        :nodes="nodes ?? []"
        :regions="regions"
        :servers="metrics.perNode"
      />
    <div class="relative flex-1 px-6 pt-6 pb-6">

      <div class="relative z-10 grid gap-6 bg-background pt-6">
        <Alert v-if="metrics.unplaced > 0" variant="info">
          <Network />{{ t('overview.unplaced', metrics.unplaced) }}
        </Alert>

        <div v-if="isPending" class="grid gap-4 sm:grid-cols-2 xl:grid-cols-5">
          <Skeleton v-for="index in 5" :key="index" class="h-28" />
        </div>
        <div v-else class="grid gap-4 sm:grid-cols-2 xl:grid-cols-5">
          <Card v-for="card in cards" :key="card.key" class="gap-2 py-4" :data-metric="card.key">
            <CardHeader class="px-4">
              <CardTitle class="flex items-center gap-1.5 text-xs font-normal text-muted-foreground">
                <component :is="card.icon" class="size-4" />{{ card.label }}
              </CardTitle>
            </CardHeader>
            <CardContent class="grid gap-2 px-4">
              <p class="text-[22px] leading-tight font-extrabold tracking-tight tabular-nums" :class="card.tone?.startsWith('text-') ? card.tone : undefined">
                {{ card.value }}
              </p>
              <div v-if="card.ratio !== undefined" class="h-1.5 overflow-hidden rounded-full bg-muted">
                <div class="h-full rounded-full transition-all" :class="card.tone ?? 'bg-primary'" :style="{ width: `${card.ratio * 100}%` }" />
              </div>
              <p class="text-xs text-muted-foreground">{{ card.detail }}</p>
            </CardContent>
          </Card>
        </div>

        <Card class="gap-0 py-0">
          <CardHeader class="flex items-center justify-between border-b py-4">
            <CardTitle>{{ t('overview.latest') }}</CardTitle>
            <Button as-child variant="link">
              <RouterLink to="/servers/list">{{ t('overview.seeAll') }}<ArrowRight /></RouterLink>
            </Button>
          </CardHeader>
          <div v-if="isPending" class="grid gap-2 p-4"><Skeleton v-for="index in 3" :key="index" class="h-10" /></div>
          <Empty v-else-if="latest.length === 0" :icon="Server" class="border-0 shadow-none">{{ t('overview.noServer') }}</Empty>
          <ul v-else class="divide-y">
            <li v-for="server in latest" :key="server.id">
              <RouterLink :to="`/servers/${server.id}`" class="flex items-center gap-3 px-5 py-3 transition-colors hover:bg-muted/50">
                <PrefabIcon :prefab="server.prefab" class="size-8 rounded-lg text-xs" />
                <span class="grid min-w-0 flex-1">
                  <span class="truncate font-semibold">{{ server.name }}</span>
                  <span class="truncate text-xs text-muted-foreground">{{ `${server.prefab.id} · ${server.node.name}` }}</span>
                </span>
                <ServerStateBadge :state="server.state" :suspended="!!server.suspended_at" />
              </RouterLink>
            </li>
          </ul>
        </Card>
      </div>
    </div>
  </div>
</template>
