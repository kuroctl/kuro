<script setup lang="ts">
// Nœud de test : accepte les essais du Studio (serveurs à durée limitée, supprimés à leur
// expiration). Enregistré dès le changement.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { useApiError } from '@/shared/api/useApiError'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Label } from '@/shared/ui/label'
import { Switch } from '@/shared/ui/switch'

import { type Node, useNodesApi } from '../nodes.api'
import { useInvalidateNodes } from '../nodes.queries'

const props = defineProps<{ node: Node; editable: boolean }>()
const { t } = useI18n()
const api = useNodesApi()
const invalidate = useInvalidateNodes()
const { handleError } = useApiError()
const pending = ref(false)

async function toggle(trials: boolean) {
  pending.value = true
  try {
    await api.update(props.node.id, { trials })
    await invalidate()
    toast.success(trials ? t('nodes.trials.enabled') : t('nodes.trials.disabled'))
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
      <CardTitle>{{ t('nodes.trials.title') }}</CardTitle>
      <CardDescription>{{ t('nodes.trials.help') }}</CardDescription>
    </CardHeader>
    <CardContent class="flex items-center gap-3">
      <Switch id="node-trials" :model-value="node.trials" :disabled="!editable || pending" @update:model-value="toggle" />
      <Label for="node-trials">{{ t('nodes.trials.label') }}</Label>
    </CardContent>
  </Card>
</template>
