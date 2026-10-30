<script setup lang="ts">
// Changements de contenu en attente du prochain démarrage : le serveur tournait, il n'a pas été
// redémarré. Le bandeau reste tant que rien n'est appliqué, avec de quoi redémarrer.
import { RotateCw } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { useApiError } from '@/shared/api/useApiError'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'

import { type Server, useServersApi } from '../servers.api'
import { useInvalidateServers } from '../servers.queries'
import { canPower } from '../state'

const props = defineProps<{ server: Server }>()

const { t } = useI18n()
const { can } = usePermissions()
const { power } = useServersApi()
const { handleError } = useApiError()
const invalidate = useInvalidateServers()
const pending = ref(false)

const summary = computed(() => {
  const { contents, removals } = props.server.pending_changes
  const parts = [
    contents > 0 ? t('servers.pending.contents', { count: contents }, contents) : '',
    removals > 0 ? t('servers.pending.removals', { count: removals }, removals) : '',
  ].filter(Boolean)
  return parts.length ? parts.join(', ') : t('servers.pending.some')
})
const running = computed(() => props.server.state === 'running' || props.server.state === 'starting')
const canRestart = computed(
  () => can('servers.power') && !props.server.suspended_at && canPower(props.server.state, 'restart'),
)

async function restart() {
  pending.value = true
  try {
    await power(props.server.id, 'restart')
    toast.success(t('servers.power.done.restart'))
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Alert v-if="server.restart_required" variant="warning" class="flex-wrap items-center" role="status">
    <RotateCw />
    <span class="flex-1">
      <strong>{{ t('servers.pending.title') }}</strong>
      {{ ` · ${running ? t('servers.pending.atRestart', { summary }) : t('servers.pending.atStart', { summary })}` }}
    </span>
    <Button v-if="running && canRestart" size="sm" variant="outline" :disabled="pending" @click="restart">
      <RotateCw />{{ t('servers.power.restart') }}
    </Button>
  </Alert>
</template>
