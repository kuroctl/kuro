<script setup lang="ts">
// Premiers pas d'une instance, sur la vue d'ensemble : chaque étape se coche d'après les données
// réelles (pas au clic), et la carte disparaît une fois tout fait ou quand l'admin la masque.
import { Check, ChevronRight, X } from '@lucide/vue'
import { useStorage } from '@vueuse/core'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import { useMailSettings } from '@/features/mail/mail.queries'
import type { Node } from '@/features/nodes/nodes.api'
import { Button } from '@/shared/ui/button'
import { Card, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'

import type { Server } from '../servers.api'

const props = defineProps<{ servers: Server[]; nodes: Node[] }>()

const { t } = useI18n()
const { can } = usePermissions()

const dismissed = useStorage('kuro.gettingStarted.dismissed', false)
const mailReadable = computed(() => can('system.settings.read'))
const { data: mail } = useMailSettings(() => mailReadable.value && !dismissed.value)

interface Step {
  key: string
  done: boolean
  to: string
}

const steps = computed<Step[]>(() => {
  const online = props.nodes.find((node) => node.status === 'online')
  const list: Step[] = [
    { key: 'node', done: props.nodes.length > 0, to: '/servers/nodes' },
    { key: 'daemon', done: Boolean(online), to: props.nodes[0] ? `/servers/nodes/${props.nodes[0].id}` : '/servers/nodes' },
    { key: 'region', done: props.nodes.some((node) => node.region), to: '/servers/regions' },
    { key: 'server', done: props.servers.length > 0, to: '/servers/new' },
  ]
  if (mailReadable.value) list.push({ key: 'mail', done: Boolean(mail.value?.configured), to: '/settings/mail' })
  return list
})

const doneCount = computed(() => steps.value.filter((step) => step.done).length)
// La lecture du mail en cours ne doit pas faire clignoter la carte.
const visible = computed(() => !dismissed.value && (!mailReadable.value || mail.value) && doneCount.value < steps.value.length)
</script>

<template>
  <Card v-if="visible" class="gap-0 py-0" data-getting-started>
    <CardHeader class="flex items-start justify-between gap-3 border-b py-4">
      <div class="grid gap-1">
        <CardTitle>{{ t('overview.gettingStarted.title') }}</CardTitle>
        <CardDescription>{{ t('overview.gettingStarted.progress', { done: doneCount, total: steps.length }, doneCount) }}</CardDescription>
      </div>
      <Button variant="ghost" size="icon-sm" :aria-label="t('overview.gettingStarted.dismiss')" @click="dismissed = true">
        <X />
      </Button>
    </CardHeader>
    <ol class="divide-y">
      <li v-for="step in steps" :key="step.key" :data-done="step.done">
        <RouterLink :to="step.to" class="flex items-center gap-3 px-5 py-3 text-sm transition-colors hover:bg-muted/50">
          <span
            class="grid size-6 shrink-0 place-items-center rounded-full border"
            :class="step.done ? 'border-success bg-success/16 text-success' : 'text-muted-foreground'"
          >
            <Check v-if="step.done" class="size-3.5" />
          </span>
          <span class="grid min-w-0 flex-1">
            <span class="font-semibold" :class="step.done ? 'text-muted-foreground line-through' : undefined">{{ t(`overview.gettingStarted.steps.${step.key}.title`) }}</span>
            <span v-if="!step.done" class="text-xs text-muted-foreground">{{ t(`overview.gettingStarted.steps.${step.key}.help`) }}</span>
          </span>
          <ChevronRight v-if="!step.done" class="size-4 shrink-0 text-muted-foreground" />
        </RouterLink>
      </li>
    </ol>
  </Card>
</template>
