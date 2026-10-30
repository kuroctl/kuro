<script setup lang="ts">
// Nombre entier ou décimal, bornes et pas du prefab.
import { useI18n } from 'vue-i18n'

import {
  NumberField,
  NumberFieldContent,
  NumberFieldDecrement,
  NumberFieldIncrement,
  NumberFieldInput,
} from '@/shared/ui/number-field'

import type { FieldControlProps } from './types'

const props = defineProps<FieldControlProps>()
const emit = defineEmits<{ 'update:modelValue': [value: unknown] }>()

const { locale } = useI18n()
const step = props.field.step ?? (props.field.type === 'int' ? 1 : 0.1)
</script>

<template>
  <NumberField
    :model-value="(modelValue as number) ?? undefined"
    :min="field.min ?? undefined"
    :max="field.max ?? undefined"
    :step="step"
    :locale="locale"
    :disabled="disabled"
    :format-options="{ maximumFractionDigits: field.type === 'int' ? 0 : 6 }"
    @update:model-value="emit('update:modelValue', $event)"
  >
    <NumberFieldContent>
      <NumberFieldDecrement />
      <NumberFieldInput :id="id" />
      <NumberFieldIncrement />
    </NumberFieldContent>
  </NumberField>
</template>
