<script setup lang="ts">
// Suppression d'un nœud en attente, hors ligne ou révoqué (ADR-0046) : confirmation, puis retour
// à la liste. Un nœud en ligne n'a pas cette carte : son daemon doit être arrêté d'abord.
import { Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { useServers } from '@/features/servers/servers.queries'
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
import { BlockedBy, type Blocker } from '@/shared/ui/blocked-by'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'

import { type Node, useNodesApi } from '../nodes.api'
import { useInvalidateNodes } from '../nodes.queries'

const props = defineProps<{ node: Node }>()
const { t } = useI18n()
const router = useRouter()
const api = useNodesApi()
const invalidate = useInvalidateNodes()
const { handleError } = useApiError()

const confirming = ref(false)
const pending = ref(false)

const BLOCKERS_SHOWN = 8
const { data: servers } = useServers()
/** Serveurs encore portés par le nœud : l'API refuserait la suppression. */
const hosted = computed(() => (servers.value ?? []).filter((server) => server.node.id === props.node.id))
const blockers = computed<Blocker[]>(() =>
  hosted.value.slice(0, BLOCKERS_SHOWN).map((server) => ({ key: server.id, label: server.name, detail: server.prefab.id, to: `/servers/${server.id}` })),
)
const more = computed(() =>
  hosted.value.length > BLOCKERS_SHOWN
    ? { label: t('common.blocked.seeAll', hosted.value.length), to: { path: '/servers/list', query: { node: props.node.id } } }
    : undefined,
)

async function remove() {
  pending.value = true
  try {
    await api.remove(props.node.id)
    confirming.value = false
    toast.success(t('nodes.delete.done', { name: props.node.display_name ?? props.node.name }))
    // La liste d'abord : la fiche supprimée ne doit pas être rechargée.
    await router.push('/servers/nodes')
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
      <CardTitle>{{ t('nodes.delete.title') }}</CardTitle>
      <CardDescription>{{ t(`nodes.delete.help.${node.status}`) }}</CardDescription>
    </CardHeader>
    <CardContent>
      <Button variant="destructive" @click="confirming = true"><Trash2 />{{ t('nodes.delete.action') }}</Button>
    </CardContent>

    <AlertDialog :open="confirming" @update:open="(open) => (confirming = open)">
      <AlertDialogContent v-if="hosted.length > 0">
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('nodes.delete.blockedTitle', { name: node.display_name ?? node.name }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('nodes.delete.blockedHelp', hosted.length) }}</AlertDialogDescription>
        </AlertDialogHeader>
        <BlockedBy :items="blockers" :more="more" />
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.close') }}</AlertDialogCancel>
        </AlertDialogFooter>
      </AlertDialogContent>
      <AlertDialogContent v-else>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('nodes.delete.confirmTitle', { name: node.display_name ?? node.name }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('nodes.delete.confirmHelp') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="remove">{{ t('nodes.delete.confirm') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </Card>
</template>
