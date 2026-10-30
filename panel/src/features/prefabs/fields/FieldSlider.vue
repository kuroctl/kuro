<script setup lang="ts">
// Curseur entre min et max ; la valeur est affichée à côté.
import { Slider } from '@/shared/ui/slider'

import type { FieldControlProps } from './types'

const props = defineProps<FieldControlProps>()
const emit = defineEmits<{ 'update:modelValue': [value: unknown] }>()
</script>

<template>
  <div class="flex items-center gap-4">
    <Slider
      :id="id"
      :model-value="[(modelValue as number) ?? props.field.min ?? 0]"
      :min="field.min ?? 0"
      :max="field.max ?? 100"
      :step="field.step ?? 1"
      :disabled="disabled"
      class="flex-1"
      @update:model-value="emit('update:modelValue', $event?.[0])"
    />
    <span class="w-12 text-right font-mono text-sm tabular-nums">{{ modelValue }}</span>
  </div>
</template>
