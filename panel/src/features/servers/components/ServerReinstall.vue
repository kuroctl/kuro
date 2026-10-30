<script setup lang="ts">
// Réinstaller (données conservées) ou tout effacer puis réinstaller (ADR-0017). Effacer exige de
// taper le nom du serveur : rien n'est récupérable ensuite.
import { RefreshCcw } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { type ReinstallMode, type Server, useServersApi } from '../servers.api'
import { useInvalidateServers } from '../servers.queries'
import { isTransitional } from '../state'

const props = defineProps<{ server: Server }>()

const { t } = useI18n()
const { reinstall } = useServersApi()
const { handleError } = useApiError()
const invalidate = useInvalidateServers()

const open = ref(false)
const mode = ref<ReinstallMode>('reinstall')
const confirmation = ref('')
const pending = ref(false)
const confirmed = computed(() => mode.value === 'reinstall' || confirmation.value === props.server.name)

function show() {
  mode.value = 'reinstall'
  confirmation.value = ''
  open.value = true
}

async function submit() {
  if (!confirmed.value) return
  pending.value = true
  try {
    await reinstall(props.server.id, mode.value)
    toast.success(t('servers.reinstall.done'))
    open.value = false
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}

const choice = (selected: boolean) =>
  selected ? 'border-primary bg-primary/5' : 'border-border hover:bg-accent/40'
</script>

<template>
  <Button size="sm" variant="outline" :disabled="isTransitional(server.state)" @click="show">
    <RefreshCcw />{{ t('servers.reinstall.action') }}
  </Button>

  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-lg">
      <DialogHeader>
        <DialogTitle>{{ t('servers.reinstall.title', { name: server.name }) }}</DialogTitle>
        <DialogDescription>{{ t('servers.reinstall.help') }}</DialogDescription>
      </DialogHeader>
      <div class="grid gap-2" role="radiogroup" :aria-label="t('servers.reinstall.title', { name: server.name })">
        <button
          type="button"
          role="radio"
          :aria-checked="mode === 'reinstall'"
          class="rounded-md border p-3 text-left text-sm"
          :class="choice(mode === 'reinstall')"
          @click="mode = 'reinstall'"
        >
          <span class="block font-medium">{{ t('servers.reinstall.keep') }}</span>
          <span class="block text-muted-foreground">{{ t('servers.reinstall.keepHelp') }}</span>
        </button>
        <button
          type="button"
          role="radio"
          :aria-checked="mode === 'reset'"
          class="rounded-md border p-3 text-left text-sm"
          :class="choice(mode === 'reset')"
          @click="mode = 'reset'"
        >
          <span class="block font-medium text-destructive">{{ t('servers.reinstall.reset') }}</span>
          <span class="block text-muted-foreground">{{ t('servers.reinstall.resetHelp') }}</span>
        </button>
      </div>
      <div v-if="mode === 'reset'" class="grid gap-2">
        <Label for="reinstall-confirm">{{ t('servers.reinstall.confirmLabel', { name: server.name }) }}</Label>
        <Input id="reinstall-confirm" v-model="confirmation" autocomplete="off" spellcheck="false" />
      </div>
      <DialogFooter>
        <Button variant="ghost" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button :variant="mode === 'reset' ? 'destructive' : 'default'" :disabled="pending || !confirmed" @click="submit">
          {{ mode === 'reset' ? t('servers.reinstall.confirmReset') : t('servers.reinstall.keep') }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
