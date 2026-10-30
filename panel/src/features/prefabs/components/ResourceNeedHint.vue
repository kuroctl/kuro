<script setup lang="ts">
// Repère sous un champ de ressource : minimum et valeur recommandée par le prefab (ADR-0031).
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Schemas } from '@/shared/api/types'

import { type NeededResource, need } from '../needs'

const props = defineProps<{
  entry: Pick<Schemas['PrefabSummaryDto'], 'resources'> | undefined
  resource: NeededResource
}>()

const { t, n } = useI18n()
const text = computed(() => {
  const declared = need(props.entry, props.resource)
  const min = declared?.min
  const recommended = declared?.recommended
  if (min != null && recommended != null) return t('prefabs.needs.both', { min: n(min), recommended: n(recommended) })
  if (min != null) return t('prefabs.needs.min', { min: n(min) })
  if (recommended != null) return t('prefabs.needs.recommended', { recommended: n(recommended) })
  return ''
})
</script>

<template>
  <p v-if="text" class="text-xs text-muted-foreground">{{ text }}</p>
</template>
