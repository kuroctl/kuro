<script setup lang="ts">
// Serveurs › Machines (ADR-0065) : machines louées chez un hébergeur et enrôlées comme nœuds.
// Louer, rendre, mettre en sommeil, réveiller ; dernier passage du ménage. Sans fournisseur
// configuré, la page envoie d'abord aux Réglages. Page volontairement simple, améliorée à l'usage.
import { Cloud, Moon, Plus, Sun, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import MachineOfferFields from '@/features/machines/components/MachineOfferFields.vue'
import { type Machine, useMachinesApi } from '@/features/machines/machines.api'
import { useInvalidateMachines, useMachines } from '@/features/machines/machines.queries'
import { formatMonthly, machineBadge } from '@/features/machines/status'
import { nodeBadge } from '@/features/nodes/status'
import ServerStateBadge from '@/features/servers/components/ServerStateBadge.vue'
import type { ServerState } from '@/features/servers/state'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

const { t, locale } = useI18n()
const { can } = usePermissions()
const { data: page, error } = useMachines()
const api = useMachinesApi()
const invalidate = useInvalidateMachines()
const { handleError, message } = useApiError()

const machines = computed(() => page.value?.machines ?? [])
const sweep = computed(() => page.value?.sweep)
const name = (machine: Machine) => machine.node_name ?? machine.id.slice(0, 8)

const renting = ref(false)
const rent = ref<{ region?: string; offer?: string }>({})
const errors = ref<FieldErrors>({})
const pending = ref<string>()
const releasing = ref<Machine>()

async function act(key: string, task: () => Promise<void>) {
  pending.value = key
  try {
    await task()
    await invalidate()
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = undefined
  }
}

async function submitRent() {
  if (!rent.value.region || !rent.value.offer) return
  pending.value = 'rent'
  errors.value = {}
  try {
    const machine = await api.rent({ region_id: rent.value.region, offer: rent.value.offer })
    renting.value = false
    rent.value = {}
    toast.success(t('machines.rented', { name: name(machine) }))
    await invalidate()
  } catch (failure) {
    errors.value = handleError(failure, ['region_id', 'offer', 'provider'])
  } finally {
    pending.value = undefined
  }
}

const sleep = (machine: Machine) => act(`sleep:${machine.id}`, async () => void (await api.sleep(machine.id)))
const wake = (machine: Machine) => act(`wake:${machine.id}`, async () => void (await api.wake(machine.id)))
const release = (machine: Machine) =>
  act(`release:${machine.id}`, async () => {
    await api.release(machine.id)
    releasing.value = undefined
    toast.success(t('machines.released', { name: name(machine) }))
  })
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.servers.machines')">
      <Button v-if="can('nodes.create') && page?.configured" @click="renting = true"><Plus />{{ t('machines.new') }}</Button>
    </PageHeader>
    <div class="grid min-w-0 gap-6 p-6">
      <p v-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <Skeleton v-else-if="!page" class="h-64" />

      <Empty v-else-if="!page.configured" :icon="Cloud" :title="t('machines.unconfigured.title')">
        {{ t('machines.unconfigured.help') }}
        <template v-if="can('system.settings.read')" #action>
          <Button size="sm" as-child><RouterLink to="/settings/machines">{{ t('machines.unconfigured.action') }}</RouterLink></Button>
        </template>
      </Empty>

      <template v-else>
        <Empty v-if="machines.length === 0" :icon="Cloud" :title="t('machines.empty.title')">{{ t('machines.empty.help') }}</Empty>
        <div v-else>
          <CardContent class="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{{ t('machines.columns.node') }}</TableHead>
                  <TableHead>{{ t('machines.columns.region') }}</TableHead>
                  <TableHead>{{ t('machines.columns.offer') }}</TableHead>
                  <TableHead>{{ t('machines.columns.state') }}</TableHead>
                  <TableHead class="text-right">{{ t('machines.columns.cost') }}</TableHead>
                  <TableHead><span class="sr-only">{{ t('machines.columns.actions') }}</span></TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow v-for="machine in machines" :key="machine.id" :data-machine="machine.id">
                  <TableCell>
                    <RouterLink v-if="machine.node_id" :to="`/servers/nodes/${machine.node_id}`" class="font-semibold hover:underline">
                      {{ name(machine) }}
                    </RouterLink>
                    <span v-else class="font-semibold">{{ name(machine) }}</span>
                    <p class="text-xs text-muted-foreground">
                      {{ [t(`machines.providers.${machine.provider}`), machine.location, machine.ipv4].filter(Boolean).join(' · ') }}
                    </p>
                    <RouterLink v-if="machine.server_id" :to="`/servers/${machine.server_id}`" class="text-xs font-semibold text-primary hover:underline">
                      {{ machine.server_name }}
                    </RouterLink>
                  </TableCell>
                  <TableCell>{{ machine.region_name ?? '—' }}</TableCell>
                  <TableCell class="font-mono text-xs">{{ machine.offer }}</TableCell>
                  <TableCell>
                    <div class="flex flex-wrap items-center gap-1.5">
                      <Badge :variant="machineBadge[machine.state]">{{ t(`machines.state.${machine.state}`) }}</Badge>
                      <Badge v-if="machine.node_status" :variant="nodeBadge[machine.node_status]">{{ t(`nodes.status.${machine.node_status}`) }}</Badge>
                      <ServerStateBadge v-if="machine.server_state" :state="machine.server_state as ServerState" />
                      <Badge v-if="machine.flag" variant="destructive">{{ t(`machines.flag.${machine.flag}`) }}</Badge>
                      <Badge v-if="machine.claim" variant="outline">{{ machine.claim }}</Badge>
                    </div>
                  </TableCell>
                  <TableCell class="text-right tabular-nums">{{ formatMonthly(machine.monthly_cost, locale) }}</TableCell>
                  <TableCell>
                    <div class="flex justify-end gap-1">
                      <Button
                        v-if="can('nodes.update') && machine.state === 'ready'"
                        variant="ghost"
                        size="sm"
                        :disabled="!!pending || machine.server_state === 'waiting_node'"
                        @click="sleep(machine)"
                      >
                        <Moon />{{ t('machines.actions.sleep') }}
                      </Button>
                      <Button v-if="can('nodes.update') && machine.state === 'asleep'" variant="ghost" size="sm" :disabled="!!pending" @click="wake(machine)">
                        <Sun />{{ t('machines.actions.wake') }}
                      </Button>
                      <Button
                        v-if="can('nodes.delete') && ['ready', 'asleep'].includes(machine.state)"
                        variant="ghost"
                        size="sm"
                        class="text-destructive"
                        :disabled="!!pending"
                        @click="releasing = machine"
                      >
                        <Trash2 />{{ t('machines.actions.release') }}
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </CardContent>
        </div>

        <Card v-if="sweep">
          <CardHeader>
            <CardTitle class="flex items-center gap-2">
              {{ t('machines.sweep.title') }}
              <Badge v-if="sweep.dry_run" variant="warning">{{ t('tasks.dryRun') }}</Badge>
            </CardTitle>
            <CardDescription>{{ t('machines.sweep.help') }}</CardDescription>
          </CardHeader>
          <CardContent class="grid gap-2 text-sm">
            <p v-if="!sweep.finished_at" class="text-muted-foreground">{{ t('tasks.never') }}</p>
            <template v-else>
              <p :class="sweep.outcome === 'error' ? 'text-destructive' : 'text-muted-foreground'">
                {{ sweep.outcome === 'error' ? sweep.error : t('tasks.lastRun', { date: new Date(sweep.finished_at).toLocaleString(locale) }) }}
              </p>
              <dl v-if="sweep.report" class="flex flex-wrap gap-x-5 gap-y-1">
                <div v-for="(value, key) in sweep.report as Record<string, unknown>" :key="key" class="flex gap-1.5">
                  <template v-if="typeof value === 'number' && value > 0">
                    <dt class="text-muted-foreground">{{ t(`machines.sweep.report.${key}`) }}</dt>
                    <dd class="font-semibold tabular-nums">{{ value }}</dd>
                  </template>
                </div>
              </dl>
            </template>
            <RouterLink v-if="can('system.settings.read')" to="/settings/tasks" class="text-[13px] font-semibold text-primary hover:underline">
              {{ t('machines.sweep.settings') }}
            </RouterLink>
          </CardContent>
        </Card>
      </template>
    </div>

    <Dialog v-model:open="renting">
      <DialogContent class="sm:max-w-2xl">
        <form class="grid gap-4" @submit.prevent="submitRent">
          <DialogHeader>
            <DialogTitle>{{ t('machines.rent.title') }}</DialogTitle>
            <DialogDescription>{{ t('machines.rent.help') }}</DialogDescription>
          </DialogHeader>
          <MachineOfferFields v-model:region="rent.region" v-model:offer="rent.offer" :errors="errors" />
          <p v-if="errors.provider" role="alert" class="text-sm text-destructive">{{ errors.provider }}</p>
          <DialogFooter>
            <Button type="button" variant="ghost" @click="renting = false">{{ t('common.cancel') }}</Button>
            <Button type="submit" :disabled="pending === 'rent' || !rent.region || !rent.offer">{{ t('machines.rent.confirm') }}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>

    <AlertDialog :open="!!releasing" @update:open="(open) => !open && (releasing = undefined)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('machines.release.title', { name: releasing ? name(releasing) : '' }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('machines.release.help') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <Button variant="destructive" :disabled="!!pending" @click="releasing && release(releasing)">{{ t('machines.release.confirm') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
