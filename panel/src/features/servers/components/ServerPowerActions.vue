<script setup lang="ts">
// Démarrer, arrêter, redémarrer ; arrêt forcé et suppression confirmés. Sans `manage` (console),
// seules les actions d'alimentation sont proposées : ni réinstallation ni suppression.
import { Play, RotateCw, Skull, Square, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { useApiError } from '@/shared/api/useApiError'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Button } from '@/shared/ui/button'

import { type PowerAction, type Server, useServersApi } from '../servers.api'
import { useInvalidateServers } from '../servers.queries'
import { canPower } from '../state'
import ServerReinstall from './ServerReinstall.vue'

const props = withDefaults(defineProps<{ server: Server; manage?: boolean }>(), { manage: true })

const { t } = useI18n()
const router = useRouter()
const { power, remove } = useServersApi()
const { handleError } = useApiError()
const invalidate = useInvalidateServers()
const pending = ref(false)
/** Un serveur suspendu ne démarre pas (ADR-0019) ; l'arrêt reste possible. */
const suspended = computed(() => !!props.server.suspended_at)
const confirm = ref<'kill' | 'delete' | null>(null)

async function run(action: PowerAction) {
  pending.value = true
  try {
    await power(props.server.id, action)
    toast.success(t(`servers.power.done.${action}`))
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
    confirm.value = null
  }
}

/** Action confirmée : lue avant toute fermeture de la fenêtre. */
function confirmed() {
  const action = confirm.value
  if (action === 'kill') void run('kill')
  else if (action === 'delete') void destroy()
}

async function destroy() {
  pending.value = true
  try {
    await remove(props.server.id)
    toast.success(t('servers.delete.done'))
    await invalidate()
    await router.push('/servers/list')
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
    confirm.value = null
  }
}
</script>

<template>
  <div class="flex flex-wrap items-center gap-2">
    <Button
      :disabled="pending || suspended || !canPower(server.state, 'start')"
      @click="run('start')"
    >
      <Play />{{ t('servers.power.start') }}
    </Button>
    <Button
      variant="secondary"
      :disabled="pending || suspended || !canPower(server.state, 'restart')"
      @click="run('restart')"
    >
      <RotateCw />{{ t('servers.power.restart') }}
    </Button>
    <Button
      variant="destructive"
      :disabled="pending || !canPower(server.state, 'stop')"
      @click="run('stop')"
    >
      <Square />{{ t('servers.power.stop') }}
    </Button>
    <Button
      variant="outline"
      :disabled="pending || !canPower(server.state, 'kill')"
      @click="confirm = 'kill'"
    >
      <Skull />{{ t('servers.power.kill') }}
    </Button>
    <template v-if="manage">
      <ServerReinstall :server="server" />
      <Button size="sm" variant="destructive" :disabled="pending || server.state === 'deleting'" @click="confirm = 'delete'">
        <Trash2 />{{ t('servers.delete.action') }}
      </Button>
    </template>

    <AlertDialog :open="confirm !== null" @update:open="(open) => !open && (confirm = null)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {{ confirm === 'kill' ? t('servers.power.killTitle') : t('servers.delete.title', { name: server.name }) }}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {{ confirm === 'kill' ? t('servers.power.killHelp') : t('servers.delete.help') }}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite, pas AlertDialogAction : celui-ci ferme la fenêtre (et efface
               l'action demandée) avant notre gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="confirmed">
            {{ confirm === 'kill' ? t('servers.power.kill') : t('servers.delete.confirm') }}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
