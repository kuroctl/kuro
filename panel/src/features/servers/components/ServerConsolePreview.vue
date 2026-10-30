<script setup lang="ts">
// Sortie de la console en lecture seule, pour la vue d'ensemble : même flux que la page Console
// (historique récent compris), sans champ de commande.
import { ArrowRight } from '@lucide/vue'
import { useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'
import { Card, CardAction, CardHeader, CardTitle } from '@/shared/ui/card'

import { type ConsoleLine, useServerConsole } from '../composables/useServerConsole'
import type { Server } from '../servers.api'
import ConsoleTerminal from './ConsoleTerminal.vue'

const props = defineProps<{ server: Server }>()

const { t } = useI18n()
const terminal = useTemplateRef<InstanceType<typeof ConsoleTerminal>>('terminal')

function onLines(lines: ConsoleLine[], dropped: number) {
  if (dropped > 0) terminal.value?.notice(t('servers.console.dropped', { count: dropped }, dropped))
  terminal.value?.write(lines.map((line) => line.text))
}

useServerConsole(() => props.server.id, onLines)
</script>

<template>
  <Card class="gap-0 overflow-hidden py-0">
    <CardHeader class="border-b py-3">
      <CardTitle>{{ t('servers.console.title') }}</CardTitle>
      <CardAction>
        <Button as-child variant="link">
          <RouterLink :to="`/servers/${server.id}/console`">{{ t('servers.overview.openConsole') }}<ArrowRight /></RouterLink>
        </Button>
      </CardAction>
    </CardHeader>
    <div class="flex h-72 flex-col bg-console">
      <ConsoleTerminal ref="terminal" :label="t('servers.console.output')" />
    </div>
  </Card>
</template>
