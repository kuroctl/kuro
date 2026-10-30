<script setup lang="ts">
// Console d'un serveur : actions d'alimentation, stats et sortie en direct, champ de commande.
import { computed, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import ConsoleCommand from '@/features/servers/components/ConsoleCommand.vue'
import ConsoleTerminal from '@/features/servers/components/ConsoleTerminal.vue'
import ServerPowerActions from '@/features/servers/components/ServerPowerActions.vue'
import ServerStatsCards from '@/features/servers/components/ServerStatsCards.vue'
import { type ConsoleLine, useServerConsole } from '@/features/servers/composables/useServerConsole'
import { useServer } from '@/features/servers/servers.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Card } from '@/shared/ui/card'
import { Skeleton } from '@/shared/ui/skeleton'

definePage({ meta: { subnav: 'server' } })

const { t } = useI18n()
const route = useRoute('/servers/[id]/console')
const { data: server } = useServer(() => route.params.id)
const terminal = useTemplateRef<InstanceType<typeof ConsoleTerminal>>('terminal')

function onLines(lines: ConsoleLine[], dropped: number) {
  if (dropped > 0) terminal.value?.notice(t('servers.console.dropped', { count: dropped }, dropped))
  terminal.value?.write(lines.map((line) => line.text))
}

const { status, samples } = useServerConsole(() => route.params.id, onLines)
const tone = computed(() => (status.value === 'live' ? 'success' : status.value === 'unavailable' ? 'destructive' : 'warning'))
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <PageHeader :title="t('servers.console.title')">
      <Badge :variant="tone" role="status">
        <span class="size-[7px] rounded-full bg-current" :class="{ 'animate-pulse': tone === 'warning' }" aria-hidden="true" />
        {{ t(`servers.console.status.${status}`) }}
      </Badge>
    </PageHeader>
    <div v-if="!server" class="grid gap-4 p-6"><Skeleton class="h-24" /><Skeleton class="h-96" /></div>
    <div v-else class="flex min-h-0 flex-1 flex-col gap-4 p-6">
      <ServerPowerActions :server="server" :manage="false" />
      <ServerStatsCards :server="server" :samples="samples" />
      <Card class="flex min-h-96 flex-1 flex-col gap-0 overflow-hidden bg-console py-0">
        <ConsoleTerminal ref="terminal" :label="t('servers.console.output')" />
        <ConsoleCommand :server="server" />
      </Card>
    </div>
  </div>
</template>
