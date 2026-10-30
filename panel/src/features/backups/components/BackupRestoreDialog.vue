<script setup lang="ts">
// Restaurer une sauvegarde : écraser les fichiers, ou tout effacer puis restaurer (ADR-0018).
// Effacer exige de taper le nom du serveur : les fichiers créés depuis sont perdus.
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import type { Server } from '@/features/servers/servers.api'
import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { type Backup, type RestoreMode, useBackupsApi } from '../backups.api'
import { useInvalidateBackups } from '../backups.queries'

const props = defineProps<{ server: Server; backup: Backup | null }>()
const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()
const { restore } = useBackupsApi()
const { handleError } = useApiError()
const invalidate = useInvalidateBackups()

const mode = ref<RestoreMode>('overwrite')
const confirmation = ref('')
const pending = ref(false)
const confirmed = computed(() => mode.value === 'overwrite' || confirmation.value === props.server.name)

watch(
  () => props.backup,
  () => {
    mode.value = 'overwrite'
    confirmation.value = ''
  },
)

async function submit() {
  if (!props.backup || !confirmed.value) return
  pending.value = true
  try {
    await restore(props.server.id, props.backup.id, mode.value)
    toast.success(t('backups.restore.done'))
    emit('close')
    await invalidate(props.server.id)
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}

const choice = (selected: boolean) => (selected ? 'border-primary bg-primary/5' : 'border-border hover:bg-accent/40')
</script>

<template>
  <Dialog :open="backup !== null" @update:open="(open) => !open && emit('close')">
    <DialogContent class="sm:max-w-lg">
      <DialogHeader>
        <DialogTitle>{{ t('backups.restore.title', { name: backup?.name ?? '' }) }}</DialogTitle>
        <DialogDescription>{{ t('backups.restore.help') }}</DialogDescription>
      </DialogHeader>
      <div class="grid gap-2" role="radiogroup" :aria-label="t('backups.restore.title', { name: backup?.name ?? '' })">
        <button
          type="button"
          role="radio"
          :aria-checked="mode === 'overwrite'"
          class="rounded-md border p-3 text-left text-sm"
          :class="choice(mode === 'overwrite')"
          @click="mode = 'overwrite'"
        >
          <span class="block font-medium">{{ t('backups.restore.overwrite') }}</span>
          <span class="block text-muted-foreground">{{ t('backups.restore.overwriteHelp') }}</span>
        </button>
        <button
          type="button"
          role="radio"
          :aria-checked="mode === 'clean'"
          class="rounded-md border p-3 text-left text-sm"
          :class="choice(mode === 'clean')"
          @click="mode = 'clean'"
        >
          <span class="block font-medium text-destructive">{{ t('backups.restore.clean') }}</span>
          <span class="block text-muted-foreground">{{ t('backups.restore.cleanHelp') }}</span>
        </button>
      </div>
      <div v-if="mode === 'clean'" class="grid gap-2">
        <Label for="restore-confirm">{{ t('backups.restore.confirmLabel', { name: server.name }) }}</Label>
        <Input id="restore-confirm" v-model="confirmation" autocomplete="off" spellcheck="false" />
      </div>
      <DialogFooter>
        <Button variant="ghost" @click="emit('close')">{{ t('common.cancel') }}</Button>
        <Button :variant="mode === 'clean' ? 'destructive' : 'default'" :disabled="pending || !confirmed" @click="submit">
          {{ mode === 'clean' ? t('backups.restore.confirmClean') : t('backups.restore.confirm') }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
