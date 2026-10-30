<script setup lang="ts">
// Mise à jour d'un serveur vers une version plus récente de son prefab (ADR-0024) : bandeau
// « mise à jour disponible », puis choix de la version, aperçu, sauvegarde préalable.
import { ArrowUpCircle } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import type { Server } from '@/features/servers/servers.api'
import { isTransitional } from '@/features/servers/state'
import { useApiError } from '@/shared/api/useApiError'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Label } from '@/shared/ui/label'
import { NativeSelect, NativeSelectOption } from '@/shared/ui/native-select'
import { Skeleton } from '@/shared/ui/skeleton'
import { Switch } from '@/shared/ui/switch'

import { useUpgradesApi } from '../upgrades.api'
import { useInvalidateUpgrades, useServerUpgrades, useUpgradePreview } from '../upgrades.queries'
import UpgradePreviewDetails from './UpgradePreviewDetails.vue'

const props = defineProps<{ server: Server }>()
const { t } = useI18n()
const { can } = usePermissions()
const { handleError } = useApiError()
const invalidate = useInvalidateUpgrades()

const { data: upgrades } = useServerUpgrades(
  () => props.server.id,
  () => props.server.prefab.version,
  () => can('servers.upgrade'),
)
const latest = computed(() => upgrades.value?.available.at(-1))

const open = ref(false)
const target = ref<string>()
const { data: preview, isFetching } = useUpgradePreview(() => props.server.id, () => (open.value ? target.value : undefined))
/** Sauvegarde : cochée par défaut, impossible si les sauvegardes du serveur sont désactivées. */
const canBackup = computed(() => (props.server.backup_limit ?? 0) > 0 && can('servers.backups.create'))
const backup = ref(true)
const pending = ref(false)

watch(open, (shown) => {
  if (!shown) return
  target.value = latest.value
  backup.value = canBackup.value
})

async function submit() {
  if (!target.value) return
  pending.value = true
  try {
    await useUpgradesApi().start(props.server.id, target.value, backup.value && canBackup.value)
    toast.success(t(backup.value && canBackup.value ? 'upgrades.startedWithBackup' : 'upgrades.started', { version: target.value }))
    open.value = false
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Alert v-if="latest" variant="info" class="flex flex-wrap items-center gap-3">
    <ArrowUpCircle />
    <span class="flex-1">{{ t('upgrades.available', { version: latest, current: server.prefab.version }) }}</span>
    <Button size="sm" :disabled="isTransitional(server.state) || !!server.suspended_at" @click="open = true">
      {{ t('upgrades.action') }}
    </Button>
  </Alert>

  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-lg">
      <DialogHeader>
        <DialogTitle>{{ t('upgrades.title', { name: server.name }) }}</DialogTitle>
        <DialogDescription>{{ t('upgrades.help') }}</DialogDescription>
      </DialogHeader>
      <div class="grid gap-4">
        <div class="grid gap-1.5">
          <Label for="upgrade-version">{{ t('upgrades.version') }}</Label>
          <NativeSelect id="upgrade-version" v-model="target">
            <NativeSelectOption v-for="version in upgrades?.available ?? []" :key="version" :value="version">
              {{ t('upgrades.versionOption', { from: server.prefab.version, to: version }) }}
            </NativeSelectOption>
          </NativeSelect>
        </div>
        <Skeleton v-if="isFetching && !preview" class="h-20" />
        <UpgradePreviewDetails v-else-if="preview" :preview="preview" />
        <div class="flex items-start gap-3">
          <Switch id="upgrade-backup" v-model="backup" :disabled="!canBackup" />
          <Label for="upgrade-backup" class="grid gap-0.5 font-normal">
            <span>{{ t('upgrades.backup') }}</span>
            <span class="text-xs text-muted-foreground">
              {{ canBackup ? t('upgrades.backupHelp') : t('upgrades.backupUnavailable') }}
            </span>
          </Label>
        </div>
      </div>
      <DialogFooter>
        <Button variant="ghost" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button :disabled="pending || !preview || preview.blockers.length > 0" @click="submit">{{ t('upgrades.confirm') }}</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
