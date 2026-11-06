<script setup lang="ts">
// « À surveiller » de la vue d'ensemble : chaque serveur ou nœud qui demande une action, avec sa
// raison et un lien vers lui ; les plus graves d'abord.
import { CircleCheck, Network } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Node } from '@/features/nodes/nodes.api'
import PrefabIcon from '@/features/prefabs/components/PrefabIcon.vue'
import { Badge } from '@/shared/ui/badge'
import { Card, CardHeader, CardTitle } from '@/shared/ui/card'

import type { Server } from '../servers.api'
import { isOverQuota } from '../state'

const LIMIT = 8

const props = defineProps<{ servers: Server[]; nodes: Node[] }>()

const { t } = useI18n()

type Reason = 'failed' | 'nodeOffline' | 'suspended' | 'overQuota'

interface Item {
  key: string
  name: string
  detail: string
  to: string
  reason: Reason
  server?: Server
}

const ORDER: Reason[] = ['failed', 'nodeOffline', 'suspended', 'overQuota']
const DANGER: Reason[] = ['failed', 'nodeOffline']

const items = computed<Item[]>(() => {
  const list: Item[] = []
  for (const server of props.servers) {
    const reason: Reason | null =
      server.state === 'failed' ? 'failed' : server.suspended_at ? 'suspended' : isOverQuota(server) ? 'overQuota' : null
    if (reason) {
      list.push({ key: `server-${server.id}`, name: server.name, detail: `${server.prefab.id} · ${server.node.name}`, to: `/servers/${server.id}`, reason, server })
    }
  }
  for (const node of props.nodes) {
    if (node.status === 'offline') {
      list.push({ key: `node-${node.id}`, name: node.name, detail: t('overview.attention.node'), to: `/servers/nodes/${node.id}`, reason: 'nodeOffline' })
    }
  }
  return list.sort((a, b) => ORDER.indexOf(a.reason) - ORDER.indexOf(b.reason))
})

const shown = computed(() => items.value.slice(0, LIMIT))
</script>

<template>
  <Card class="gap-0 py-0" data-attention>
    <CardHeader class="border-b py-4">
      <CardTitle>{{ t('overview.attention.title') }}</CardTitle>
    </CardHeader>
    <p v-if="items.length === 0" class="flex items-center gap-2 px-5 py-4 text-sm text-success">
      <CircleCheck class="size-4" />{{ t('overview.attention.allClear') }}
    </p>
    <ul v-else class="divide-y">
      <li v-for="item in shown" :key="item.key">
        <RouterLink :to="item.to" class="flex items-center gap-3 px-5 py-3 transition-colors hover:bg-muted/50">
          <PrefabIcon v-if="item.server" :prefab="item.server.prefab" class="size-8 rounded-lg text-xs" />
          <span v-else class="grid size-8 place-items-center rounded-lg bg-muted text-muted-foreground"><Network class="size-4" /></span>
          <span class="grid min-w-0 flex-1">
            <span class="truncate font-semibold">{{ item.name }}</span>
            <span class="truncate text-xs text-muted-foreground">{{ item.detail }}</span>
          </span>
          <Badge :variant="DANGER.includes(item.reason) ? 'destructive' : 'warning'">{{ t(`overview.attention.reasons.${item.reason}`) }}</Badge>
        </RouterLink>
      </li>
    </ul>
    <RouterLink v-if="items.length > LIMIT" to="/servers/list" class="border-t px-5 py-3 text-sm text-muted-foreground hover:text-foreground">
      {{ t('overview.attention.more', items.length - LIMIT) }}
    </RouterLink>
  </Card>
</template>
