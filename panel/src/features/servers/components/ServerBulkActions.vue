<script setup lang="ts">
// Actions groupées sur la sélection (ADR-0025) : confirmation qui résume l'effet, puis résultat
// serveur par serveur. Seules les actions que l'admin détient sont proposées ; l'API revérifie.
import { ArrowUpCircle, Ban, ChevronDown, CircleCheck, DatabaseBackup, Play, Power, RotateCw, Skull, Square } from '@lucide/vue'
import type { Component } from 'vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/shared/ui/dropdown-menu'
import { Label } from '@/shared/ui/label'
import { Switch } from '@/shared/ui/switch'

import { type BulkAction, type BulkOutcome, useServersApi } from '../servers.api'
import { useInvalidateServers } from '../servers.queries'

const props = defineProps<{
  /** Nombre de serveurs visés. */
  count: number
  /** Identifiants visés, résolus au moment d'agir (tous les résultats du filtre, au besoin). */
  resolve: () => Promise<string[]>
  /** Noms connus, pour le résultat. */
  names: Record<string, string>
}>()
const emit = defineEmits<{ done: [] }>()

const { t, te } = useI18n()
const { can } = usePermissions()
const { handleError } = useApiError()
const invalidate = useInvalidateServers()

interface Entry {
  action: BulkAction
  icon: Component
  permission: string
  destructive?: boolean
}
const ENTRIES: Entry[][] = [
  [
    { action: 'start', icon: Play, permission: 'servers.power' },
    { action: 'restart', icon: RotateCw, permission: 'servers.power' },
    { action: 'stop', icon: Square, permission: 'servers.power' },
    { action: 'kill', icon: Skull, permission: 'servers.power', destructive: true },
  ],
  [
    { action: 'upgrade', icon: ArrowUpCircle, permission: 'servers.upgrade' },
    { action: 'backup', icon: DatabaseBackup, permission: 'servers.backups.create' },
  ],
  [
    { action: 'suspend', icon: Ban, permission: 'servers.suspend', destructive: true },
    { action: 'unsuspend', icon: CircleCheck, permission: 'servers.suspend' },
  ],
]
const groups = computed(() => ENTRIES.map((group) => group.filter((entry) => can(entry.permission))).filter((g) => g.length))

const chosen = ref<Entry>()
const backup = ref(true)
const pending = ref(false)
const outcomes = ref<BulkOutcome[]>()
const failures = computed(() => outcomes.value?.filter((o) => !o.ok) ?? [])
const succeeded = computed(() => (outcomes.value?.length ?? 0) - failures.value.length)

function choose(entry: Entry) {
  chosen.value = entry
  backup.value = can('servers.backups.create')
  outcomes.value = undefined
}

function reason(code: string | null | undefined): string {
  const key = `errors.${code}`
  return code && te(key) ? t(key) : (code ?? '')
}

async function run() {
  if (!chosen.value) return
  pending.value = true
  try {
    const ids = await props.resolve()
    const action = chosen.value.action
    outcomes.value = await useServersApi().bulk(action, ids, action === 'upgrade' && backup.value)
    if (!failures.value.length) {
      toast.success(t('servers.bulk.done', { count: succeeded.value }))
      chosen.value = undefined
    }
    await invalidate()
    emit('done')
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <DropdownMenu v-if="groups.length">
    <DropdownMenuTrigger as-child>
      <Button size="sm" class="h-8"><Power />{{ t('servers.bulk.actions') }}<ChevronDown /></Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" class="w-52">
      <template v-for="(group, index) in groups" :key="index">
        <DropdownMenuSeparator v-if="index > 0" />
        <DropdownMenuItem
          v-for="entry in group"
          :key="entry.action"
          :class="entry.destructive ? 'text-destructive focus:text-destructive' : undefined"
          @select="choose(entry)"
        >
          <component :is="entry.icon" />{{ t(`servers.bulk.action.${entry.action}`) }}
        </DropdownMenuItem>
      </template>
    </DropdownMenuContent>
  </DropdownMenu>

  <Dialog :open="!!chosen" @update:open="(open) => !open && (chosen = undefined)">
    <DialogContent v-if="chosen" class="sm:max-w-lg">
      <DialogHeader>
        <DialogTitle>{{ t(`servers.bulk.action.${chosen.action}`) }}</DialogTitle>
        <DialogDescription>{{ t(`servers.bulk.confirm.${chosen.action}`, { count }) }}</DialogDescription>
      </DialogHeader>
      <div v-if="chosen.action === 'upgrade' && !outcomes" class="flex items-start gap-3">
        <Switch id="bulk-backup" v-model="backup" :disabled="!can('servers.backups.create')" />
        <Label for="bulk-backup" class="grid gap-0.5 font-normal">
          <span>{{ t('upgrades.backup') }}</span>
          <span class="text-xs text-muted-foreground">{{ t('servers.bulk.upgradeHelp') }}</span>
        </Label>
      </div>
      <div v-if="outcomes" class="grid gap-2 text-sm" role="status">
        <p>{{ t('servers.bulk.result', { ok: succeeded, failed: failures.length }) }}</p>
        <ul class="grid max-h-60 gap-1 overflow-y-auto rounded-lg border px-3 py-2">
          <li v-for="failure in failures" :key="failure.server_id" class="flex gap-2">
            <span class="font-medium">{{ names[failure.server_id] ?? failure.server_id.slice(0, 8) }}</span>
            <span class="text-muted-foreground">{{ reason(failure.error) }}</span>
          </li>
        </ul>
      </div>
      <DialogFooter>
        <Button variant="ghost" @click="chosen = undefined">{{ outcomes ? t('common.close') : t('common.cancel') }}</Button>
        <Button v-if="!outcomes" :variant="chosen.destructive ? 'destructive' : 'default'" :disabled="pending" @click="run">
          {{ t('servers.bulk.run', { count }) }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
