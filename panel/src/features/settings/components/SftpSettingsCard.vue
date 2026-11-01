<script setup lang="ts">
// Accès SFTP (ADR-0047) : ouvert ou coupé pour tout le projet, dès le changement. Coupé, toute
// connexion est refusée et les sessions ouvertes ferment au bail suivant (une minute au plus).
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { useApiError } from '@/shared/api/useApiError'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Label } from '@/shared/ui/label'
import { Switch } from '@/shared/ui/switch'

import { type PlatformSettings, useSettingsApi } from '../settings.api'
import { useInvalidateSettings } from '../settings.queries'

defineProps<{ settings: PlatformSettings; editable: boolean }>()
const { t } = useI18n()
const api = useSettingsApi()
const invalidate = useInvalidateSettings()
const { handleError } = useApiError()
const pending = ref(false)

async function toggle(enabled: boolean) {
  pending.value = true
  try {
    await api.update({ sftp_enabled: enabled })
    await invalidate()
    toast.success(enabled ? t('settings.sftp.enabled') : t('settings.sftp.disabled'))
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ t('settings.sftp.title') }}</CardTitle>
      <CardDescription>{{ t('settings.sftp.help') }}</CardDescription>
    </CardHeader>
    <CardContent>
      <div class="flex items-center gap-3">
        <Switch
          id="sftp-enabled"
          :model-value="settings.sftp_enabled"
          :disabled="!editable || pending"
          @update:model-value="toggle"
        />
        <Label for="sftp-enabled">{{ t('settings.sftp.label') }}</Label>
      </div>
    </CardContent>
  </Card>
</template>
