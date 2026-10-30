<script setup lang="ts">
// Repère sous un champ de ressource : minimum et valeur recommandée par le prefab (ADR-0031).
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Schemas } from '@/shared/api/types'
import { formatCores } from '@/shared/lib/format'

import { type NeededResource, need } from '../needs'

const props = defineProps<{
  entry: Pick<Schemas['PrefabSummaryDto'], 'resources'> | undefined
  resource: NeededResource
}>()

const { t, n, locale } = useI18n()
/** Le CPU est déclaré en pourcent (100 = un cœur), affiché en cœurs. */
const amount = (value: number) => (props.resource === 'cpu_percent' ? formatCores(value, locale.value) : n(value))
const text = computed(() => {
  const declared = need(props.entry, props.resource)
  const min = declared?.min
  const recommended = declared?.recommended
  if (min != null && recommended != null) return t('prefabs.needs.both', { min: amount(min), recommended: amount(recommended) })
  if (min != null) return t('prefabs.needs.min', { min: amount(min) })
  if (recommended != null) return t('prefabs.needs.recommended', { recommended: amount(recommended) })
  return ''
})
</script>

<template>
  <p v-if="text" class="text-xs text-muted-foreground">{{ text }}</p>
</template>
