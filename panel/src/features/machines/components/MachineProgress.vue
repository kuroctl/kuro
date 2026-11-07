<script setup lang="ts">
// Installation d'une machine louée (ADR-0065) : étape courante, chronologie dépliable, erreur
// en rouge avec sa cause (sortie de l'installeur, ou aucune nouvelle de la machine).
import { ChevronDown } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Machine } from '@/features/machines/machines.api'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/shared/ui/collapsible'

const props = defineProps<{ machine: Machine }>()
const { t, locale } = useI18n()

const steps = computed(() => props.machine.progress)
const current = computed(() => steps.value.at(-1))
const cause = (error: string) => (error === 'no_news' ? t('machines.progress.noNews') : error)
const time = (at: string) => new Date(at).toLocaleString(locale.value)
</script>

<template>
  <Collapsible v-if="current" class="text-xs" data-machine-progress>
    <CollapsibleTrigger class="flex items-center gap-1 text-left hover:underline" :class="current.error ? 'text-destructive' : 'text-muted-foreground'">
      {{ current.error ? t('machines.progress.failedStep', { step: t(`machines.progress.steps.${current.step}`) }) : t(`machines.progress.steps.${current.step}`) }}
      <ChevronDown class="size-3.5" />
    </CollapsibleTrigger>
    <CollapsibleContent>
      <ol class="mt-1.5 grid gap-1 border-l border-border pl-2.5">
        <li v-for="step in steps" :key="step.step">
          <span :class="step.error ? 'font-semibold text-destructive' : ''">{{ t(`machines.progress.steps.${step.step}`) }}</span>
          <span class="ml-1.5 text-muted-foreground">{{ time(step.at) }}</span>
          <p v-if="step.error" role="alert" class="text-destructive">{{ cause(step.error) }}</p>
        </li>
      </ol>
    </CollapsibleContent>
  </Collapsible>
</template>
