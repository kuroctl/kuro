<script setup lang="ts">
// Résultat d'une exécution : badge d'état et cause traduite (raison propre à la planification,
// ou code d'erreur de l'API).
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { Badge, type BadgeVariants } from '@/shared/ui/badge'

import type { ScheduleRun } from '../schedules.api'

const props = defineProps<{ run: ScheduleRun; detail?: boolean }>()
const { t, te } = useI18n()

const VARIANTS: Record<ScheduleRun['outcome'], BadgeVariants['variant']> = {
  running: 'info',
  succeeded: 'success',
  failed: 'destructive',
  skipped: 'secondary',
}

const reason = computed(() => {
  const code = props.run.reason
  if (!code) return null
  const text = te(`schedules.runs.reasons.${code}`)
    ? t(`schedules.runs.reasons.${code}`)
    : te(`errors.${code}`)
      ? t(`errors.${code}`)
      : t('errors.unknown')
  return props.run.failed_step != null ? t('schedules.runs.failedStep', { n: props.run.failed_step + 1, reason: text }) : text
})
</script>

<template>
  <span class="inline-flex flex-wrap items-center gap-2">
    <Badge :variant="VARIANTS[run.outcome]">
      <span class="size-[7px] rounded-full bg-current" :class="{ 'animate-pulse': run.outcome === 'running' }" aria-hidden="true" />
      {{ t(`schedules.runs.outcomes.${run.outcome}`) }}
    </Badge>
    <span v-if="detail && reason" class="text-xs text-muted-foreground">{{ reason }}</span>
  </span>
</template>
