<script setup lang="ts">
// Enrôlement d'un nœud en attente ou révoqué : un nouveau jeton invalide les précédents. Le
// ticket n'est affiché que jusqu'au prochain changement de page.
import { RefreshCw } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'

import { type Node, type NodeTicket, useNodesApi } from '../nodes.api'
import { useInvalidateNodes } from '../nodes.queries'
import EnrollmentTicket from './EnrollmentTicket.vue'

const props = defineProps<{ node: Node }>()
const { t } = useI18n()
const api = useNodesApi()
const { handleError } = useApiError()
const invalidate = useInvalidateNodes()

const pending = ref(false)
const ticket = ref<NodeTicket>()

async function reissue() {
  pending.value = true
  try {
    ticket.value = await api.reissueToken(props.node.id)
    await invalidate()
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
      <CardTitle>{{ t('nodes.enroll.title') }}</CardTitle>
      <CardDescription>{{ node.status === 'revoked' ? t('nodes.enroll.revokedHelp') : t('nodes.enroll.pendingHelp') }}</CardDescription>
    </CardHeader>
    <CardContent class="grid gap-4">
      <EnrollmentTicket v-if="ticket" :ticket="ticket.enrollment" />
      <div>
        <Button :variant="ticket ? 'outline' : 'default'" :disabled="pending" @click="reissue">
          <RefreshCw />{{ ticket ? t('nodes.enroll.again') : t('nodes.enroll.issue') }}
        </Button>
      </div>
    </CardContent>
  </Card>
</template>
