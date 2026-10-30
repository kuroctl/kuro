<script setup lang="ts">
// Serveur MCP (ADR-0023) : ouvert ou coupé aux agents IA dès le changement, sans redémarrage.
// Coupé, les jetons déjà émis sont refusés ; les accès accordés restent, prêts à resservir.
import { Copy } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Label } from '@/shared/ui/label'
import { Switch } from '@/shared/ui/switch'

import { type PlatformSettings, useSettingsApi } from '../settings.api'
import { useInvalidateSettings } from '../settings.queries'

/** Clé de configuration de l'API qui active le MCP. */
const CONFIG_KEY = 'mcp.public_url'

const props = defineProps<{ settings: PlatformSettings; editable: boolean }>()
const { t } = useI18n()
const api = useSettingsApi()
const invalidate = useInvalidateSettings()
const { handleError } = useApiError()
const pending = ref(false)

async function toggle(enabled: boolean) {
  pending.value = true
  try {
    await api.update({ mcp_enabled: enabled })
    await invalidate()
    toast.success(enabled ? t('settings.mcp.enabled') : t('settings.mcp.disabled'))
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}

async function copy() {
  if (!props.settings.mcp_url) return
  await navigator.clipboard.writeText(props.settings.mcp_url)
  toast.success(t('settings.mcp.copied'))
}
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ t('settings.mcp.title') }}</CardTitle>
      <CardDescription>{{ t('settings.mcp.help') }}</CardDescription>
    </CardHeader>
    <CardContent class="grid gap-4">
      <div class="flex items-center gap-3">
        <!-- Non configuré : on peut encore le couper, pas l'ouvrir. -->
        <Switch
          id="mcp-enabled"
          :model-value="settings.mcp_enabled"
          :disabled="!editable || pending || (!settings.mcp_url && !settings.mcp_enabled)"
          @update:model-value="toggle"
        />
        <Label for="mcp-enabled">{{ t('settings.mcp.label') }}</Label>
      </div>
      <div v-if="settings.mcp_url" class="grid gap-1.5">
        <p class="text-xs text-muted-foreground">{{ t('settings.mcp.url') }}</p>
        <div class="flex items-center gap-2">
          <code class="min-w-0 flex-1 truncate rounded-md border bg-muted px-3 py-2 font-mono text-xs">{{ settings.mcp_url }}</code>
          <Button type="button" variant="outline" size="icon" :aria-label="t('settings.mcp.copy')" @click="copy">
            <Copy class="size-4" />
          </Button>
        </div>
      </div>
      <p v-else class="rounded-lg border border-warning/35 bg-warning/8 px-4 py-3 text-sm">
        {{ t('settings.mcp.notConfigured', { key: CONFIG_KEY }) }}
      </p>
    </CardContent>
  </Card>
</template>
