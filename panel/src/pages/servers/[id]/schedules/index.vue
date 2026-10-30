<script setup lang="ts">
// Tâches planifiées d'un serveur (ADR-0022) : déclencheur en clair, prochaine et dernière
// exécution, activation, lancement immédiat, modification et suppression.
import { Archive, CalendarClock, MoreHorizontal, Pencil, Play, Plus, Power, Terminal, Trash2 } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import RunOutcome from '@/features/schedules/components/RunOutcome.vue'
import { type Schedule, type ScheduleStep, useSchedulesApi } from '@/features/schedules/schedules.api'
import { useInvalidateSchedules, useSchedules } from '@/features/schedules/schedules.queries'
import { describeTrigger } from '@/features/schedules/trigger'
import { useApiError } from '@/shared/api/useApiError'
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
import { Button } from '@/shared/ui/button'
import { Card, CardContent } from '@/shared/ui/card'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/shared/ui/dropdown-menu'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'
import { Switch } from '@/shared/ui/switch'

definePage({ meta: { subnav: 'server' } })

const STEP_ICONS = { command: Terminal, power: Power, backup: Archive } as const

const { t, locale } = useI18n()
const route = useRoute('/servers/[id]/schedules/')
const id = route.params.id
const { can } = usePermissions()
const { data: schedules, isPending } = useSchedules(id, () => can('servers.schedules.read'))
const api = useSchedulesApi()
const invalidate = useInvalidateSchedules()
const { handleError } = useApiError()

const deleting = ref<Schedule>()
const pending = ref(false)

/** Date dans le fuseau de la tâche, suivie de ce fuseau. */
function when(date: string, timezone: string): string {
  const text = new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short', timeZone: timezone }).format(new Date(date))
  return `${text} (${timezone})`
}

function stepLabel(step: ScheduleStep): string {
  if (step.action === 'power') return t(`servers.power.${step.value}`)
  return step.value ?? t('schedules.editor.actionsList.backup')
}

function request(schedule: Schedule, enabled: boolean) {
  const { name, cron, timezone, only_when_online, steps } = schedule
  return { name, cron, timezone, only_when_online, steps, enabled }
}

async function toggle(schedule: Schedule, enabled: boolean) {
  try {
    await api.update(id, schedule.id, request(schedule, enabled))
    await invalidate(id)
  } catch (error) {
    handleError(error)
  }
}

async function runNow(schedule: Schedule) {
  try {
    await api.run(id, schedule.id)
    toast.success(t('schedules.started', { name: schedule.name }))
    await invalidate(id)
  } catch (error) {
    handleError(error)
  }
}

async function remove() {
  const schedule = deleting.value
  if (!schedule) return
  pending.value = true
  try {
    await api.remove(id, schedule.id)
    toast.success(t('schedules.delete.done', { name: schedule.name }))
    await invalidate(id)
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
    deleting.value = undefined
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('schedules.title')">
      <Button v-if="can('servers.schedules.manage')" as-child>
        <RouterLink :to="`/servers/${id}/schedules/new`"><Plus />{{ t('schedules.add') }}</RouterLink>
      </Button>
    </PageHeader>
    <div class="grid max-w-4xl gap-4 p-6">
      <p class="text-sm text-muted-foreground">{{ t('schedules.help') }}</p>
      <p v-if="!can('servers.schedules.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('schedules.forbidden') }}
      </p>
      <div v-else-if="isPending" class="grid gap-3"><Skeleton v-for="index in 2" :key="index" class="h-32" /></div>
      <Empty v-else-if="!schedules?.length" :icon="CalendarClock" :title="t('schedules.empty')">{{ t('schedules.emptyHelp') }}</Empty>
      <Card v-for="schedule in schedules" v-else :key="schedule.id" class="py-4" :data-schedule="schedule.name">
        <CardContent class="grid gap-3 px-5">
          <div class="flex flex-wrap items-start gap-3">
            <div class="grid min-w-0 flex-1 gap-0.5">
              <RouterLink :to="`/servers/${id}/schedules/${schedule.id}`" class="truncate text-[15px] font-bold hover:underline">
                {{ schedule.name }}
              </RouterLink>
              <p class="text-[13px] text-muted-foreground">{{ describeTrigger(schedule.cron, locale, t) }}</p>
            </div>
            <Switch
              v-if="can('servers.schedules.manage')"
              :model-value="schedule.enabled"
              :aria-label="t('schedules.enable', { name: schedule.name })"
              @update:model-value="(value: boolean) => toggle(schedule, value)"
            />
          </div>

          <ul class="flex flex-wrap gap-2">
            <li
              v-for="(step, index) in schedule.steps"
              :key="index"
              class="inline-flex max-w-full items-center gap-1.5 rounded-md bg-muted px-2 py-1 text-xs"
            >
              <component :is="STEP_ICONS[step.action]" class="size-3.5 shrink-0 text-muted-foreground" />
              <span class="truncate font-mono">{{ stepLabel(step) }}</span>
            </li>
          </ul>

          <div class="flex flex-wrap items-center gap-x-5 gap-y-2 text-[13px] text-muted-foreground">
            <span v-if="schedule.running" class="font-semibold text-info">{{ t('schedules.running') }}</span>
            <span v-else-if="schedule.enabled && schedule.next_run_at">{{ t('schedules.next', { date: when(schedule.next_run_at, schedule.timezone) }) }}</span>
            <span v-else>{{ t('schedules.disabled') }}</span>
            <span v-if="schedule.last_run" class="inline-flex flex-wrap items-center gap-2">
              {{ t('schedules.last', { date: when(schedule.last_run.started_at, schedule.timezone) }) }}
              <RunOutcome :run="schedule.last_run" detail />
            </span>
            <span v-else>{{ t('schedules.never') }}</span>
            <div v-if="can('servers.schedules.manage')" class="ml-auto flex items-center gap-1">
              <Button variant="secondary" size="sm" :disabled="schedule.running" @click="runNow(schedule)"><Play />{{ t('schedules.runNow') }}</Button>
              <DropdownMenu>
                <DropdownMenuTrigger as-child>
                  <Button variant="ghost" size="icon-sm" :aria-label="t('schedules.actions', { name: schedule.name })"><MoreHorizontal /></Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem as-child>
                    <RouterLink :to="`/servers/${id}/schedules/${schedule.id}`"><Pencil />{{ t('schedules.edit') }}</RouterLink>
                  </DropdownMenuItem>
                  <DropdownMenuItem variant="destructive" @select="deleting = schedule"><Trash2 />{{ t('schedules.delete.action') }}</DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>

    <AlertDialog :open="!!deleting" @update:open="(open) => !open && (deleting = undefined)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('schedules.delete.title', { name: deleting?.name ?? '' }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('schedules.delete.help') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="remove">{{ t('schedules.delete.action') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
