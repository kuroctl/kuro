<script setup lang="ts">
// Sauvegardes d'un serveur (ADR-0018) : liste, création, verrou, téléchargement, restauration,
// suppression. Chaque action n'apparaît qu'avec sa permission ; l'API reste seule juge.
import { Archive, Download, History, Lock, LockOpen, MoreHorizontal, Plus, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { type Backup, useBackupsApi } from '@/features/backups/backups.api'
import { useBackups, useInvalidateBackups } from '@/features/backups/backups.queries'
import BackupRestoreDialog from '@/features/backups/components/BackupRestoreDialog.vue'
import { submitDownload } from '@/features/backups/download'
import { useServer } from '@/features/servers/servers.queries'
import { isTransitional } from '@/features/servers/state'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { formatBytes } from '@/shared/lib/format'
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/shared/ui/dropdown-menu'
import { Empty } from '@/shared/ui/empty'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Skeleton } from '@/shared/ui/skeleton'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

definePage({ meta: { subnav: 'server' } })

const { t, locale } = useI18n()
const route = useRoute('/servers/[id]/backups')
const id = computed(() => route.params.id)
const { data: server } = useServer(id)
const { data: list, error } = useBackups(id)
const api = useBackupsApi()
const { handleError, message } = useApiError()
const invalidate = useInvalidateBackups()
const { can } = usePermissions()

const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'short', timeStyle: 'short' }))
const ready = computed(() => list.value?.backups.filter((backup) => backup.state === 'ready').length ?? 0)
const running = computed(() => list.value?.backups.some((backup) => backup.state === 'creating') ?? false)
const canCreate = computed(
  () => can('servers.backups.create') && (list.value?.limit ?? 0) > 0 && !running.value && !!server.value && !isTransitional(server.value.state),
)
const hasActions = computed(() =>
  ['servers.backups.create', 'servers.backups.restore', 'servers.backups.delete', 'servers.backups.download'].some(can),
)

const creating = ref(false)
const name = ref('')
const nameError = ref<string>()
const pending = ref(false)
const restoring = ref<Backup | null>(null)
const deleting = ref<Backup | null>(null)

function openCreate() {
  name.value = ''
  nameError.value = undefined
  creating.value = true
}

async function create() {
  pending.value = true
  try {
    await api.create(id.value, name.value.trim() || undefined)
    toast.success(t('backups.create.done'))
    creating.value = false
    await invalidate(id.value)
  } catch (failure) {
    nameError.value = handleError(failure, ['name']).name
  } finally {
    pending.value = false
  }
}

async function toggleLock(backup: Backup) {
  try {
    await api.setLocked(id.value, backup.id, !backup.locked)
    await invalidate(id.value)
  } catch (failure) {
    handleError(failure)
  }
}

async function download(backup: Backup) {
  try {
    const { ticket } = await api.downloadTicket(id.value, backup.id)
    submitDownload(id.value, backup.id, ticket)
  } catch (failure) {
    handleError(failure)
  }
}

async function confirmDelete() {
  const backup = deleting.value
  if (!backup) return
  pending.value = true
  try {
    await api.remove(id.value, backup.id)
    toast.success(t('backups.delete.done', { name: backup.name }))
    deleting.value = null
    await invalidate(id.value)
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = false
  }
}

const stateVariant = (backup: Backup) => (backup.state === 'failed' ? 'destructive' : backup.state === 'ready' ? 'secondary' : 'outline')
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <PageHeader :title="t('backups.title')">
      <span v-if="list" class="text-sm text-muted-foreground tabular-nums">{{ t('backups.count', { count: ready, limit: list.limit }) }}</span>
      <Button v-if="can('servers.backups.create')" size="sm" :disabled="!canCreate" @click="openCreate"><Plus />{{ t('backups.create.action') }}</Button>
    </PageHeader>
    <div class="grid gap-4 p-6">
      <p class="text-sm text-muted-foreground">{{ t('backups.help') }}</p>
      <p v-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <div v-else-if="!list" class="grid gap-2"><Skeleton v-for="index in 3" :key="index" class="h-9" /></div>
      <p v-else-if="list.limit === 0" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t('backups.disabled') }}</p>
      <Empty v-else-if="list.backups.length === 0" :icon="Archive">{{ t('backups.empty') }}</Empty>
      <Table v-else>
        <TableHeader>
          <TableRow>
            <TableHead>{{ t('backups.columns.name') }}</TableHead>
            <TableHead class="w-36">{{ t('backups.columns.state') }}</TableHead>
            <TableHead class="w-28 text-right">{{ t('backups.columns.size') }}</TableHead>
            <TableHead class="w-40">{{ t('backups.columns.created') }}</TableHead>
            <TableHead v-if="hasActions" class="w-10" />
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow v-for="backup in list.backups" :key="backup.id">
            <TableCell>
              <span class="flex items-center gap-2">
                <Lock v-if="backup.locked" class="size-3.5 text-muted-foreground" :aria-label="t('backups.locked')" />
                <span>{{ backup.name }}</span>
              </span>
              <span v-if="backup.failure" class="block text-xs text-destructive">{{ backup.failure }}</span>
            </TableCell>
            <TableCell>
              <Badge :variant="stateVariant(backup)">{{ t(`backups.state.${backup.state}`) }}</Badge>
            </TableCell>
            <TableCell class="text-right tabular-nums text-muted-foreground">
              {{ backup.size_bytes != null ? formatBytes(backup.size_bytes, locale) : '' }}
            </TableCell>
            <TableCell class="text-muted-foreground">{{ dates.format(new Date(backup.created_at)) }}</TableCell>
            <TableCell v-if="hasActions">
              <DropdownMenu v-if="backup.state !== 'creating'">
                <DropdownMenuTrigger as-child>
                  <Button size="icon" variant="ghost" class="size-8" :aria-label="t('backups.actions', { name: backup.name })"><MoreHorizontal /></Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <template v-if="backup.state === 'ready'">
                    <DropdownMenuItem v-if="can('servers.backups.download')" @select="download(backup)">
                      <Download />{{ t('backups.download') }}
                    </DropdownMenuItem>
                    <DropdownMenuItem v-if="can('servers.backups.create')" @select="toggleLock(backup)">
                      <component :is="backup.locked ? LockOpen : Lock" />{{ backup.locked ? t('backups.unlock') : t('backups.lock') }}
                    </DropdownMenuItem>
                    <DropdownMenuItem v-if="can('servers.backups.restore')" @select="restoring = backup">
                      <History />{{ t('backups.restore.action') }}
                    </DropdownMenuItem>
                  </template>
                  <DropdownMenuItem v-if="can('servers.backups.delete')" variant="destructive" :disabled="backup.locked" @select="deleting = backup">
                    <Trash2 />{{ t('backups.delete.action') }}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </div>

    <Dialog v-model:open="creating">
      <DialogContent class="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{{ t('backups.create.title') }}</DialogTitle>
          <DialogDescription>{{ t('backups.create.help') }}</DialogDescription>
        </DialogHeader>
        <form class="grid gap-2" @submit.prevent="create">
          <Label for="backup-name">{{ t('backups.create.name') }}</Label>
          <Input id="backup-name" v-model="name" maxlength="64" :placeholder="t('backups.create.placeholder')" :aria-invalid="!!nameError" />
          <p v-if="nameError" class="text-sm text-destructive">{{ nameError }}</p>
          <DialogFooter class="mt-2">
            <Button type="button" variant="ghost" @click="creating = false">{{ t('common.cancel') }}</Button>
            <Button type="submit" :disabled="pending">{{ t('backups.create.submit') }}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>

    <BackupRestoreDialog v-if="server" :server="server" :backup="restoring" @close="restoring = null" />

    <AlertDialog :open="deleting !== null" @update:open="(open) => !open && (deleting = null)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('backups.delete.title', { name: deleting?.name ?? '' }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('backups.delete.help') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button variant="outline" @click="deleting = null">{{ t('common.cancel') }}</Button>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="confirmDelete">{{ t('backups.delete.confirm') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
