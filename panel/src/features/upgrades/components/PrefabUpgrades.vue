<script setup lang="ts">
// Mise à jour groupée (ADR-0024) : serveurs de ce prefab sur une version plus ancienne que
// celle affichée, avec l'aperçu de chacun ; ceux qui ne sont pas bloqués sont cochés.
import { ArrowUpCircle } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { useApiError } from '@/shared/api/useApiError'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Label } from '@/shared/ui/label'
import { Switch } from '@/shared/ui/switch'

import { useUpgradesApi } from '../upgrades.api'
import { useInvalidateUpgrades, useUpgradeCandidates } from '../upgrades.queries'
import UpgradePreviewDetails from './UpgradePreviewDetails.vue'

const props = defineProps<{ prefab: string; version: string }>()
const { t } = useI18n()
const { can } = usePermissions()
const { handleError } = useApiError()
const invalidate = useInvalidateUpgrades()

const { data: candidates } = useUpgradeCandidates(() => props.prefab, () => props.version, () => can('servers.upgrade'))
const selected = ref(new Set<string>())
watch(candidates, (list) => {
  selected.value = new Set((list ?? []).filter((c) => !c.preview.blockers.length).map((c) => c.server_id))
}, { immediate: true })
function choose(id: string, on: boolean) {
  const next = new Set(selected.value)
  if (on) next.add(id)
  else next.delete(id)
  selected.value = next
}

const canBackup = computed(() => can('servers.backups.create'))
const backup = ref(true)
const pending = ref(false)

async function submit() {
  pending.value = true
  try {
    const results = await useUpgradesApi().startMany([...selected.value], props.version, backup.value && canBackup.value)
    const started = results.filter((r) => r.started).length
    const refused = results.length - started
    if (started) toast.success(t('upgrades.bulk.started', { count: started, version: props.version }))
    if (refused) toast.error(t('upgrades.bulk.refused', { count: refused }))
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Card v-if="candidates?.length">
    <CardHeader>
      <CardTitle class="flex items-center gap-2"><ArrowUpCircle class="size-4 text-info" />{{ t('upgrades.bulk.title', { version }) }}</CardTitle>
      <CardDescription>{{ t('upgrades.bulk.help') }}</CardDescription>
    </CardHeader>
    <CardContent class="grid gap-4">
      <ul class="grid gap-3">
        <li v-for="candidate in candidates" :key="candidate.server_id" class="grid gap-2 rounded-lg border px-4 py-3">
          <div class="flex items-center gap-3">
            <Switch
              :id="`upgrade-${candidate.server_id}`"
              :model-value="selected.has(candidate.server_id)"
              :disabled="candidate.preview.blockers.length > 0"
              @update:model-value="(on: boolean) => choose(candidate.server_id, on)"
            />
            <Label :for="`upgrade-${candidate.server_id}`" class="font-semibold">{{ candidate.name }}</Label>
            <Badge variant="outline" class="font-mono">{{ t('upgrades.versionOption', { from: candidate.version, to: version }) }}</Badge>
            <Badge v-if="candidate.preview.blockers.length" variant="destructive" class="ml-auto">{{ t('upgrades.bulk.blocked') }}</Badge>
          </div>
          <UpgradePreviewDetails :preview="candidate.preview" />
        </li>
      </ul>
      <div class="flex flex-wrap items-center gap-3">
        <Switch id="bulk-upgrade-backup" v-model="backup" :disabled="!canBackup" />
        <Label for="bulk-upgrade-backup" class="font-normal">{{ t('upgrades.bulk.backup') }}</Label>
        <Button class="ml-auto" :disabled="pending || selected.size === 0" @click="submit">
          {{ t('upgrades.bulk.confirm', { count: selected.size }) }}
        </Button>
      </div>
    </CardContent>
  </Card>
</template>
