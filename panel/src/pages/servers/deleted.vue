<script setup lang="ts">
// Serveurs supprimés dont le nœud garde encore les sauvegardes (ADR-0059) : un serveur du même
// nœud peut les reprendre, pour les restaurer ensuite depuis sa page.
import { ArchiveRestore } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { type DeletedServer, useServersApi } from '@/features/servers/servers.api'
import { useDeletedServers, useInvalidateServers, useServers } from '@/features/servers/servers.queries'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { formatBytes } from '@/shared/lib/format'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Empty } from '@/shared/ui/empty'
import { Label } from '@/shared/ui/label'
import { NativeSelect, NativeSelectOption } from '@/shared/ui/native-select'
import { Skeleton } from '@/shared/ui/skeleton'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

const { t, locale } = useI18n()
const { can } = usePermissions()
const { data: deleted, isPending } = useDeletedServers(() => can('servers.backups.read'))
const { data: servers } = useServers()
const api = useServersApi()
const invalidate = useInvalidateServers()
const { handleError } = useApiError()

const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }))
const size = (server: DeletedServer) => formatBytes(server.backups.reduce((sum, b) => sum + (b.size_bytes ?? 0), 0), locale.value)

const adopting = ref<DeletedServer>()
const target = ref('')
const pending = ref(false)
/** Seuls les serveurs du nœud qui garde les archives peuvent les reprendre. */
const candidates = computed(() => (servers.value ?? []).filter((s) => s.node.id === adopting.value?.node_id))

function open(server: DeletedServer) {
  adopting.value = server
  target.value = ''
}

async function adopt() {
  const from = adopting.value
  if (!from || !target.value) return
  pending.value = true
  try {
    const { adopted } = await api.adopt(from.id, target.value)
    toast.success(t('deletedServers.adopt.done', { count: adopted }, adopted))
    adopting.value = undefined
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('deletedServers.title')" />
    <div class="grid min-w-0 gap-4 p-6">
      <p class="max-w-3xl text-sm text-muted-foreground">{{ t('deletedServers.help') }}</p>
      <p v-if="!can('servers.backups.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('deletedServers.forbidden') }}
      </p>
      <div v-else-if="isPending" class="grid gap-2"><Skeleton v-for="index in 3" :key="index" class="h-12" /></div>
      <Empty v-else-if="!deleted?.length" :icon="ArchiveRestore">{{ t('deletedServers.empty') }}</Empty>
      <div v-else class="rounded-xl border bg-card shadow-card">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{{ t('deletedServers.server') }}</TableHead>
              <TableHead>{{ t('deletedServers.node') }}</TableHead>
              <TableHead>{{ t('deletedServers.owner') }}</TableHead>
              <TableHead>{{ t('deletedServers.backups') }}</TableHead>
              <TableHead>{{ t('deletedServers.deletedAt') }}</TableHead>
              <TableHead>{{ t('deletedServers.purgeAt') }}</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="server in deleted" :key="server.id">
              <TableCell>
                <p class="font-medium">{{ server.name }}</p>
                <p class="text-xs text-muted-foreground">{{ server.prefab_id }} {{ server.prefab_version }}</p>
              </TableCell>
              <TableCell>{{ server.node_name }}</TableCell>
              <TableCell class="text-muted-foreground">{{ server.owner_ref ?? '—' }}</TableCell>
              <TableCell>{{ t('deletedServers.count', { count: server.backups.length, size: size(server) }, server.backups.length) }}</TableCell>
              <TableCell>{{ dates.format(new Date(server.deleted_at)) }}</TableCell>
              <TableCell>{{ dates.format(new Date(server.purge_at)) }}</TableCell>
              <TableCell class="text-right">
                <Button v-if="can('servers.backups.restore')" variant="outline" size="sm" @click="open(server)">
                  <ArchiveRestore />{{ t('deletedServers.adopt.open') }}
                </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </div>
    </div>

    <Dialog :open="!!adopting" @update:open="(value) => !value && (adopting = undefined)">
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{{ t('deletedServers.adopt.title', { name: adopting?.name ?? '' }) }}</DialogTitle>
          <DialogDescription>{{ t('deletedServers.adopt.help', { node: adopting?.node_name ?? '' }) }}</DialogDescription>
        </DialogHeader>
        <p v-if="!candidates.length" class="text-sm text-muted-foreground">{{ t('deletedServers.adopt.none') }}</p>
        <div v-else class="grid gap-1.5">
          <Label for="adopt-target">{{ t('deletedServers.adopt.target') }}</Label>
          <NativeSelect id="adopt-target" v-model="target">
            <NativeSelectOption value="" disabled>{{ t('deletedServers.adopt.pick') }}</NativeSelectOption>
            <NativeSelectOption v-for="server in candidates" :key="server.id" :value="server.id">{{ server.name }}</NativeSelectOption>
          </NativeSelect>
        </div>
        <DialogFooter>
          <Button variant="outline" @click="adopting = undefined">{{ t('common.cancel') }}</Button>
          <Button :disabled="!target || pending" @click="adopt">{{ t('deletedServers.adopt.confirm') }}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </div>
</template>
