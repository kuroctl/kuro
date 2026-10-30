<script setup lang="ts">
// Oui / non.
import { Switch } from '@/shared/ui/switch'

import type { FieldControlProps } from './types'

defineProps<FieldControlProps>()
const emit = defineEmits<{ 'update:modelValue': [value: unknown] }>()
</script>

<template>
  <Switch
    :id="id"
    :model-value="Boolean(modelValue)"
    :disabled="disabled"
    @update:model-value="emit('update:modelValue', $event)"
  />
</template>
