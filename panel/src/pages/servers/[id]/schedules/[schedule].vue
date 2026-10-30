<script setup lang="ts">
// Création (`new`) ou modification d'une tâche planifiée : nom, déclencheur, étapes, puis
// l'historique de ses exécutions. Enregistrement par la barre des modifications.
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { schedulableActions } from '@/features/prefabs/manifest'
import { usePrefab } from '@/features/prefabs/prefabs.queries'
import RunOutcome from '@/features/schedules/components/RunOutcome.vue'
import StepsEditor from '@/features/schedules/components/StepsEditor.vue'
import TriggerEditor from '@/features/schedules/components/TriggerEditor.vue'
import { type Schedule, type ScheduleRequest, type ScheduleStep, useSchedulesApi } from '@/features/schedules/schedules.api'
import { useInvalidateSchedules, useSchedule, useScheduleRuns } from '@/features/schedules/schedules.queries'
import { defaultTrigger, fromCron, timezones, toCron, type TriggerForm } from '@/features/schedules/trigger'
import { useServer } from '@/features/servers/servers.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Skeleton } from '@/shared/ui/skeleton'
import { Switch } from '@/shared/ui/switch'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'
import { UnsavedBar } from '@/shared/ui/unsaved-bar'

definePage({ meta: { subnav: 'server' } })

const STEP_PERMISSIONS = {
  command: 'servers.console.write',
  power: 'servers.power',
  backup: 'servers.backups.create',
  action: 'servers.actions.run',
} as const

const { t, locale } = useI18n()
const route = useRoute('/servers/[id]/schedules/[schedule]')
const router = useRouter()
const server = route.params.id
const scheduleId = computed(() => (route.params.schedule === 'new' ? undefined : route.params.schedule))
const { can } = usePermissions()
const { data: schedule, isPending } = useSchedule(server, scheduleId)
const { data: runs } = useScheduleRuns(server, scheduleId)
const api = useSchedulesApi()
const invalidate = useInvalidateSchedules()
const { handleError } = useApiError()

const editable = computed(() => can('servers.schedules.manage'))
// Actions planifiables du prefab du serveur (ADR-0039), avec leur libellé.
const { data: serverData } = useServer(server)
const { data: prefab } = usePrefab(
  () => serverData.value?.prefab.id,
  () => serverData.value?.prefab.version,
)
const prefabLocale = usePrefabLocale(() => serverData.value?.prefab)
const prefabActions = computed(() =>
  schedulableActions(prefab.value?.manifest).map((key) => ({ key, label: prefabLocale.actionLabel(key) })),
)
const allowed = computed(() =>
  (Object.keys(STEP_PERMISSIONS) as ScheduleStep['action'][]).filter(
    (action) => can(STEP_PERMISSIONS[action]) && (action !== 'action' || prefabActions.value.length > 0),
  ),
)

const form = reactive({ name: '', enabled: true, onlyWhenOnline: false, timezone: timezones()[0] ?? 'UTC' })
const trigger = ref<TriggerForm>(defaultTrigger())
const steps = ref<ScheduleStep[]>([])
const errors = ref<FieldErrors>({})
const pending = ref(false)

/** Nouvelle tâche ouverte depuis une action du prefab (`?action=cron`) : une étape préremplie. */
const prefilled = computed(() => (typeof route.query.action === 'string' ? route.query.action : undefined))

function load(value?: Schedule) {
  form.name = value?.name ?? (prefilled.value ? prefabLocale.actionLabel(prefilled.value) : '')
  form.enabled = value?.enabled ?? true
  form.onlyWhenOnline = value?.only_when_online ?? true
  form.timezone = value?.timezone ?? timezones()[0] ?? 'UTC'
  trigger.value = value ? fromCron(value.cron) : defaultTrigger()
  steps.value = value
    ? value.steps.map((step) => ({ ...step }))
    : prefilled.value
      ? [{ action: 'action', value: prefilled.value, delay_secs: 0, continue_on_failure: false }]
      : []
  errors.value = {}
}
watch(schedule, (value) => value && load(value), { immediate: true })

const request = computed<ScheduleRequest>(() => ({
  name: form.name.trim(),
  cron: toCron(trigger.value),
  timezone: form.timezone,
  enabled: form.enabled,
  only_when_online: form.onlyWhenOnline,
  steps: steps.value,
}))

const dirty = computed(() => {
  if (!scheduleId.value) return form.name.trim() !== '' || steps.value.length > 0
  const saved = schedule.value
  if (!saved) return false
  const original: ScheduleRequest = {
    name: saved.name,
    cron: saved.cron,
    timezone: saved.timezone,
    enabled: saved.enabled,
    only_when_online: saved.only_when_online,
    steps: saved.steps,
  }
  return JSON.stringify(request.value) !== JSON.stringify(original)
})

async function save() {
  pending.value = true
  errors.value = {}
  try {
    if (scheduleId.value) {
      load(await api.update(server, scheduleId.value, request.value))
      toast.success(t('schedules.saved'))
    } else {
      const created = await api.create(server, request.value)
      toast.success(t('schedules.created', { name: created.name }))
      await router.replace(`/servers/${server}/schedules/${created.id}`)
    }
    await invalidate(server)
  } catch (error) {
    const fields = ['name', 'cron', 'timezone', 'steps', ...steps.value.flatMap((_, i) => [`steps.${i}.value`, `steps.${i}.delay_secs`])]
    errors.value = handleError(error, fields)
  } finally {
    pending.value = false
  }
}

const when = (date: string) =>
  new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'medium', timeZone: form.timezone }).format(new Date(date))
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="scheduleId ? (schedule?.name ?? '') : t('schedules.editor.newTitle')" />
    <div v-if="scheduleId && isPending" class="grid max-w-4xl gap-4 p-6"><Skeleton class="h-64" /></div>
    <form v-else class="grid max-w-4xl gap-6 p-6" @submit.prevent="save">
      <Card>
        <CardHeader>
          <CardTitle>{{ t('schedules.editor.general') }}</CardTitle>
        </CardHeader>
        <CardContent class="grid gap-4">
          <div class="grid gap-1.5">
            <Label for="schedule-name">{{ t('schedules.editor.name') }}</Label>
            <Input id="schedule-name" v-model="form.name" :disabled="!editable" :aria-invalid="!!errors.name" />
            <p v-if="errors.name" class="text-xs text-destructive">{{ errors.name }}</p>
          </div>
          <label class="flex items-center justify-between gap-3 text-sm">
            <span>{{ t('schedules.editor.enabled') }}</span>
            <Switch v-model="form.enabled" :disabled="!editable" :aria-label="t('schedules.editor.enabled')" />
          </label>
          <label class="flex items-center justify-between gap-3 text-sm">
            <span class="grid">
              {{ t('schedules.editor.onlyWhenOnline') }}
              <span class="text-xs text-muted-foreground">{{ t('schedules.editor.onlyWhenOnlineHelp') }}</span>
            </span>
            <Switch v-model="form.onlyWhenOnline" :disabled="!editable" :aria-label="t('schedules.editor.onlyWhenOnline')" />
          </label>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{{ t('schedules.editor.trigger') }}</CardTitle>
        </CardHeader>
        <CardContent>
          <TriggerEditor v-model="trigger" v-model:timezone="form.timezone" :server="server" :disabled="!editable" :errors="errors" />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{{ t('schedules.editor.steps') }}</CardTitle>
          <CardDescription>{{ t('schedules.editor.stepsHelp') }}</CardDescription>
        </CardHeader>
        <CardContent>
          <StepsEditor v-model="steps" :allowed="allowed" :actions="prefabActions" :disabled="!editable" :errors="errors" />
        </CardContent>
      </Card>

      <UnsavedBar
        v-if="editable"
        :show="dirty"
        :pending="pending"
        :can-save="steps.length > 0"
        :save-label="scheduleId ? undefined : t('schedules.editor.create')"
        @reset="load(schedule)"
      />

      <Card v-if="scheduleId" class="gap-0 pb-0">
        <CardHeader class="pb-4">
          <CardTitle>{{ t('schedules.runs.title') }}</CardTitle>
        </CardHeader>
        <p v-if="!runs?.length" class="px-5 pb-5 text-sm text-muted-foreground">{{ t('schedules.runs.empty') }}</p>
        <Table v-else class="rounded-t-none">
          <TableHeader>
            <TableRow>
              <TableHead>{{ t('schedules.runs.started') }}</TableHead>
              <TableHead>{{ t('schedules.runs.trigger') }}</TableHead>
              <TableHead>{{ t('schedules.runs.outcome') }}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="run in runs" :key="run.id">
              <TableCell class="tabular-nums">{{ when(run.started_at) }}</TableCell>
              <TableCell>{{ t(`schedules.runs.triggers.${run.trigger}`) }}</TableCell>
              <TableCell><RunOutcome :run="run" detail /></TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </Card>
    </form>
  </div>
</template>
