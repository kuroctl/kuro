<script setup lang="ts">
// Vue d'ensemble d'un serveur : état, actions, sortie de la console (si l'admin peut la lire),
// adresses (nom DNS s'il existe), ressources.
import { Ban, CircleAlert, Gamepad2, Network, TriangleAlert } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import { usePermissions } from '@/features/account/account.queries'
import ServerAddressesCard from '@/features/dns/components/ServerAddressesCard.vue'
import PrefabCover from '@/features/prefabs/components/PrefabCover.vue'
import PrefabIcon from '@/features/prefabs/components/PrefabIcon.vue'
import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { belowMinimum } from '@/features/prefabs/needs'
import { useCatalogEntry } from '@/features/prefabs/prefabs.queries'
import ServerConsolePreview from '@/features/servers/components/ServerConsolePreview.vue'
import ServerDiskUsage from '@/features/servers/components/ServerDiskUsage.vue'
import ServerActions from '@/features/servers/components/ServerActions.vue'
import ServerPowerActions from '@/features/servers/components/ServerPowerActions.vue'
import PendingRestartBanner from '@/features/servers/components/PendingRestartBanner.vue'
import { useServer } from '@/features/servers/servers.queries'
import { isOverQuota } from '@/features/servers/state'
import ServerUpgrade from '@/features/upgrades/components/ServerUpgrade.vue'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { formatCores } from '@/shared/lib/format'
import { Alert } from '@/shared/ui/alert'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'
import { Skeleton } from '@/shared/ui/skeleton'

definePage({ meta: { subnav: 'server' } })

const { t, locale: lang } = useI18n()
const route = useRoute('/servers/[id]/')
const { data: server } = useServer(() => route.params.id)
const locale = usePrefabLocale(() => server.value?.prefab)
const { can } = usePermissions()
const entry = useCatalogEntry(() => server.value?.prefab)
/** Ressources sous le minimum du jeu (ADR-0031) : gardées, mais le serveur risque de ne pas démarrer. */
const underMinimum = computed(() => (server.value ? belowMinimum(entry.value, server.value) : []))
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="server?.name ?? ''">
      <template v-if="server" #meta>
        <span class="inline-flex items-center gap-1.5"><Gamepad2 class="size-4" />{{ locale.prefabName() }}</span>
        <span class="inline-flex items-center gap-1.5"><Network class="size-4" />{{ server.node.name }}</span>
      </template>
    </PageHeader>
    <div v-if="!server" class="grid gap-4 p-6"><Skeleton class="h-40" /></div>
    <div v-else class="grid gap-6 p-6">
      <PrefabCover :prefab="server.prefab">
        <div class="flex min-h-32 items-end gap-4 p-5">
          <PrefabIcon :prefab="server.prefab" class="size-14 rounded-xl text-xl ring-card" />
          <div class="min-w-0">
            <p class="truncate text-lg font-bold">{{ locale.prefabName() }}</p>
            <p class="font-mono text-xs text-muted-foreground">{{ `${server.prefab.id} · ${server.prefab.version}` }}</p>
          </div>
        </div>
      </PrefabCover>
      <Alert v-if="server.suspended_at" variant="destructive"><Ban />{{ t('servers.suspension.banner') }}</Alert>
      <Alert v-else-if="isOverQuota(server)" variant="warning"><TriangleAlert />{{ t('servers.disk.overQuota') }}</Alert>
      <Alert v-if="underMinimum.length" variant="warning">
        <TriangleAlert />{{ t('servers.needs.below', { resources: underMinimum.map((resource) => t(`servers.fields.${resource}`)).join(', ') }) }}
      </Alert>
      <ServerUpgrade :server="server" />
      <ServerPowerActions :server="server" />
      <ServerActions :server="server" />
      <PendingRestartBanner :server="server" />
      <Alert v-if="server.state === 'failed' && server.state_reason" variant="destructive">
        <CircleAlert />{{ t('servers.failedReason', { reason: server.state_reason }) }}
      </Alert>
      <ServerConsolePreview v-if="can('servers.console.read')" :server="server" />
      <div class="grid gap-6 md:grid-cols-3">
        <ServerAddressesCard :server="server" />
        <Card>
          <CardHeader><CardTitle>{{ t('servers.overview.resources') }}</CardTitle></CardHeader>
          <CardContent class="grid gap-1 text-sm">
            <span>{{ t('servers.overview.memory', { value: server.memory_mb }) }}</span>
            <span>{{ t('servers.overview.cpu', { value: formatCores(server.cpu_percent, lang) }) }}</span>
            <ServerDiskUsage :server="server" class="mt-1" />
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{{ t('servers.overview.prefab') }}</CardTitle></CardHeader>
          <CardContent class="grid gap-1 text-sm">
            <span>{{ locale.prefabName() }}</span>
            <span class="font-mono text-xs text-muted-foreground">{{ `${server.prefab.id} · ${server.prefab.version}` }}</span>
            <span class="text-muted-foreground">{{ t('servers.overview.node', { node: server.node.name }) }}</span>
          </CardContent>
        </Card>
      </div>
    </div>
  </div>
</template>
