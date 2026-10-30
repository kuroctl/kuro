<script setup lang="ts">
// Étapes d'une tâche : action (selon les permissions détenues), valeur, attente, poursuite en cas
// d'échec ; ajout, retrait et réordonnancement. 10 étapes au plus.
import { ArrowDown, ArrowUp, Plus, Trash2 } from '@lucide/vue'
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { NumberField, NumberFieldContent, NumberFieldDecrement, NumberFieldIncrement, NumberFieldInput } from '@/shared/ui/number-field'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

import type { ScheduleStep } from '../schedules.api'

const MAX_STEPS = 10
const POWER = ['start', 'stop', 'restart', 'kill'] as const

const props = defineProps<{ allowed: ScheduleStep['action'][]; disabled?: boolean; errors: Record<string, string> }>()
const steps = defineModel<ScheduleStep[]>({ required: true })
const { t, locale } = useI18n()

function defaultValue(action: ScheduleStep['action']): string | null {
  return action === 'power' ? 'restart' : action === 'command' ? '' : null
}

function update(index: number, patch: Partial<ScheduleStep>) {
  steps.value = steps.value.map((step, i) => (i === index ? { ...step, ...patch } : step))
}

function setAction(index: number, action: ScheduleStep['action']) {
  update(index, { action, value: defaultValue(action) })
}

function add() {
  const action = props.allowed[0]
  if (!action) return
  steps.value = [...steps.value, { action, value: defaultValue(action), delay_secs: 0, continue_on_failure: false }]
}

function move(index: number, by: number) {
  const list = [...steps.value]
  const [step] = list.splice(index, 1)
  list.splice(index + by, 0, step!)
  steps.value = list
}

function remove(index: number) {
  steps.value = steps.value.filter((_, i) => i !== index)
}
</script>

<template>
  <div class="grid gap-3">
    <ol class="grid gap-3">
      <li v-for="(step, index) in steps" :key="index" class="grid gap-3 rounded-xl border bg-muted/40 p-4" :data-step="index">
        <div class="flex items-center gap-2">
          <span class="text-[11px] font-bold tracking-[0.07em] text-muted-foreground uppercase">{{ t('schedules.editor.step', { n: index + 1 }) }}</span>
          <div v-if="!disabled" class="ml-auto flex gap-1">
            <Button type="button" variant="ghost" size="icon-sm" :disabled="index === 0" :aria-label="t('schedules.editor.up', { n: index + 1 })" @click="move(index, -1)">
              <ArrowUp />
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              :disabled="index === steps.length - 1"
              :aria-label="t('schedules.editor.down', { n: index + 1 })"
              @click="move(index, 1)"
            >
              <ArrowDown />
            </Button>
            <Button type="button" variant="ghost" size="icon-sm" :aria-label="t('schedules.editor.remove', { n: index + 1 })" @click="remove(index)">
              <Trash2 />
            </Button>
          </div>
        </div>
        <div class="grid gap-3 sm:grid-cols-[12rem_1fr_9rem]">
          <div class="grid content-start gap-1.5">
            <Label :for="`step-${index}-action`">{{ t('schedules.editor.action') }}</Label>
            <Select :model-value="step.action" :disabled="disabled" @update:model-value="(action) => setAction(index, action as ScheduleStep['action'])">
              <SelectTrigger :id="`step-${index}-action`" class="w-full">
                <SelectValue>{{ t(`schedules.editor.actionsList.${step.action}`) }}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem v-for="action in allowed" :key="action" :value="action">{{ t(`schedules.editor.actionsList.${action}`) }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div class="grid content-start gap-1.5">
            <template v-if="step.action === 'power'">
              <Label :for="`step-${index}-value`">{{ t('schedules.editor.power') }}</Label>
              <Select :model-value="step.value ?? 'restart'" :disabled="disabled" @update:model-value="(value) => update(index, { value: String(value) })">
                <SelectTrigger :id="`step-${index}-value`" class="w-full" :aria-invalid="!!errors[`steps.${index}.value`]">
                  <SelectValue>{{ t(`servers.power.${step.value ?? 'restart'}`) }}</SelectValue>
                </SelectTrigger>
                <SelectContent>
                  <SelectItem v-for="action in POWER" :key="action" :value="action">{{ t(`servers.power.${action}`) }}</SelectItem>
                </SelectContent>
              </Select>
            </template>
            <template v-else>
              <Label :for="`step-${index}-value`">{{ step.action === 'command' ? t('schedules.editor.command') : t('schedules.editor.backupName') }}</Label>
              <Input
                :id="`step-${index}-value`"
                :model-value="step.value ?? ''"
                :class="{ 'font-mono': step.action === 'command' }"
                :disabled="disabled"
                :aria-invalid="!!errors[`steps.${index}.value`]"
                @update:model-value="(value) => update(index, { value: String(value) || (step.action === 'backup' ? null : '') })"
              />
            </template>
            <p v-if="errors[`steps.${index}.value`]" class="text-xs text-destructive">{{ errors[`steps.${index}.value`] }}</p>
          </div>
          <div class="grid content-start gap-1.5">
            <Label :for="`step-${index}-delay`">{{ t('schedules.editor.delay') }}</Label>
            <NumberField
              :model-value="step.delay_secs"
              :min="0"
              :max="3600"
              :step="5"
              :step-snapping="false"
              :locale="locale"
              :disabled="disabled"
              @update:model-value="(delay) => update(index, { delay_secs: Number.isNaN(delay) ? 0 : delay })"
            >
              <NumberFieldContent>
                <NumberFieldDecrement />
                <NumberFieldInput :id="`step-${index}-delay`" :aria-invalid="!!errors[`steps.${index}.delay_secs`]" />
                <NumberFieldIncrement />
              </NumberFieldContent>
            </NumberField>
            <p v-if="errors[`steps.${index}.delay_secs`]" class="text-xs text-destructive">{{ errors[`steps.${index}.delay_secs`] }}</p>
          </div>
        </div>
        <label class="inline-flex w-fit cursor-pointer items-center gap-2 text-sm">
          <input
            type="checkbox"
            :checked="step.continue_on_failure"
            :disabled="disabled"
            @change="update(index, { continue_on_failure: ($event.target as HTMLInputElement).checked })"
          >
          {{ t('schedules.editor.continueOnFailure') }}
        </label>
      </li>
    </ol>
    <p v-if="errors.steps" class="text-xs text-destructive">{{ errors.steps }}</p>
    <p v-if="!disabled && allowed.length === 0" class="text-sm text-muted-foreground">{{ t('schedules.editor.noPermission') }}</p>
    <Button v-else-if="!disabled" type="button" variant="outline" class="w-fit" :disabled="steps.length >= MAX_STEPS" @click="add">
      <Plus />{{ t('schedules.editor.addStep') }}
    </Button>
  </div>
</template>
