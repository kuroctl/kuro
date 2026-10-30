<script setup lang="ts">
// Déclencheur d'une tâche : raccourcis (toutes les X heures, chaque jour, certains jours) ou
// expression cron, fuseau horaire, et aperçu des prochaines exécutions calculé par l'API.
import { watchDebounced } from '@vueuse/core'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { NumberField, NumberFieldContent, NumberFieldDecrement, NumberFieldIncrement, NumberFieldInput } from '@/shared/ui/number-field'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

import { useSchedulesApi } from '../schedules.api'
import { type TriggerForm, type TriggerMode, timezones, toCron, WEEK, weekdayName } from '../trigger'

const props = defineProps<{ server: string; disabled?: boolean; errors: Record<string, string> }>()
const form = defineModel<TriggerForm>({ required: true })
const timezone = defineModel<string>('timezone', { required: true })
const { t, locale } = useI18n()
const api = useSchedulesApi()

const MODES: TriggerMode[] = ['hourly', 'daily', 'weekly', 'custom']
const zones = timezones()

const time = computed({
  get: () => `${String(form.value.hour).padStart(2, '0')}:${String(form.value.minute).padStart(2, '0')}`,
  set: (value: string) => {
    const [hour, minute] = value.split(':').map(Number)
    if (Number.isInteger(hour) && Number.isInteger(minute)) form.value = { ...form.value, hour: hour!, minute: minute! }
  },
})

function setDay(day: number, checked: boolean) {
  const days = checked ? [...new Set([...form.value.days, day])] : form.value.days.filter((d) => d !== day)
  form.value = { ...form.value, days }
}

// Aperçu : prochaines échéances lues par l'API dans le fuseau choisi.
const preview = ref<string[] | null>(null)
watchDebounced(
  () => [toCron(form.value), timezone.value] as const,
  async ([cron, zone]) => {
    try {
      preview.value = (await api.preview(props.server, { cron, timezone: zone })).runs
    } catch {
      preview.value = null
    }
  },
  { debounce: 300, immediate: true },
)
const format = (date: string) =>
  new Intl.DateTimeFormat(locale.value, { dateStyle: 'full', timeStyle: 'short', timeZone: timezone.value }).format(new Date(date))
</script>

<template>
  <div class="grid gap-4">
    <div class="grid gap-4 sm:grid-cols-2">
      <div class="grid content-start gap-1.5">
        <Label for="schedule-mode">{{ t('schedules.editor.mode') }}</Label>
        <Select :model-value="form.mode" :disabled="disabled" @update:model-value="(mode) => (form = { ...form, mode: mode as TriggerMode, cron: toCron(form) })">
          <SelectTrigger id="schedule-mode" class="w-full">
            <SelectValue>{{ t(`schedules.editor.modes.${form.mode}`) }}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem v-for="mode in MODES" :key="mode" :value="mode">{{ t(`schedules.editor.modes.${mode}`) }}</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div class="grid content-start gap-1.5">
        <Label for="schedule-timezone">{{ t('schedules.editor.timezone') }}</Label>
        <Select v-model="timezone" :disabled="disabled">
          <SelectTrigger id="schedule-timezone" class="w-full" :aria-invalid="!!errors.timezone">
            <SelectValue>{{ timezone }}</SelectValue>
          </SelectTrigger>
          <SelectContent class="max-h-72">
            <SelectItem v-for="zone in zones" :key="zone" :value="zone">{{ zone }}</SelectItem>
          </SelectContent>
        </Select>
        <p v-if="errors.timezone" class="text-xs text-destructive">{{ errors.timezone }}</p>
      </div>
    </div>

    <div v-if="form.mode === 'hourly'" class="grid gap-4 sm:grid-cols-2">
      <div class="grid gap-1.5">
        <Label for="schedule-every">{{ t('schedules.editor.every') }}</Label>
        <NumberField :model-value="form.every" :min="1" :max="23" :disabled="disabled" @update:model-value="(every) => (form = { ...form, every: every || 1 })">
          <NumberFieldContent>
            <NumberFieldDecrement />
            <NumberFieldInput id="schedule-every" />
            <NumberFieldIncrement />
          </NumberFieldContent>
        </NumberField>
      </div>
      <div class="grid gap-1.5">
        <Label for="schedule-minute">{{ t('schedules.editor.minute') }}</Label>
        <NumberField :model-value="form.minute" :min="0" :max="59" :disabled="disabled" @update:model-value="(minute) => (form = { ...form, minute: minute || 0 })">
          <NumberFieldContent>
            <NumberFieldDecrement />
            <NumberFieldInput id="schedule-minute" />
            <NumberFieldIncrement />
          </NumberFieldContent>
        </NumberField>
      </div>
    </div>

    <div v-else-if="form.mode === 'daily' || form.mode === 'weekly'" class="grid gap-4 sm:grid-cols-[10rem_1fr]">
      <div class="grid content-start gap-1.5">
        <Label for="schedule-time">{{ t('schedules.editor.time') }}</Label>
        <Input id="schedule-time" v-model="time" type="time" :disabled="disabled" />
      </div>
      <fieldset v-if="form.mode === 'weekly'" class="grid content-start gap-1.5">
        <legend class="mb-1.5 text-sm font-medium">{{ t('schedules.editor.days') }}</legend>
        <div class="flex flex-wrap gap-2">
          <label
            v-for="day in WEEK"
            :key="day"
            class="inline-flex cursor-pointer items-center gap-2 rounded-lg border bg-muted px-3 py-2 text-sm font-semibold has-checked:border-primary/35 has-checked:bg-accent"
          >
            <input type="checkbox" :checked="form.days.includes(day)" :disabled="disabled" @change="setDay(day, ($event.target as HTMLInputElement).checked)">
            {{ weekdayName(day, locale) }}
          </label>
        </div>
      </fieldset>
    </div>

    <div v-else class="grid gap-1.5">
      <Label for="schedule-cron">{{ t('schedules.editor.cron') }}</Label>
      <Input
        id="schedule-cron"
        :model-value="form.cron"
        class="font-mono"
        :disabled="disabled"
        :aria-invalid="!!errors.cron"
        @update:model-value="(cron) => (form = { ...form, cron: String(cron) })"
      />
      <p v-if="errors.cron" class="text-xs text-destructive">{{ errors.cron }}</p>
      <p v-else class="text-xs text-muted-foreground">{{ t('schedules.editor.cronHelp') }}</p>
    </div>

    <div class="rounded-lg bg-muted px-4 py-3" aria-live="polite">
      <p class="text-[11px] font-bold tracking-[0.07em] text-muted-foreground uppercase">{{ t('schedules.editor.preview') }}</p>
      <ul v-if="preview" class="mt-1 grid gap-0.5 text-sm">
        <li v-for="run in preview" :key="run">{{ format(run) }}</li>
      </ul>
      <p v-else class="mt-1 text-sm text-destructive">{{ t('schedules.editor.previewInvalid') }}</p>
    </div>
  </div>
</template>
