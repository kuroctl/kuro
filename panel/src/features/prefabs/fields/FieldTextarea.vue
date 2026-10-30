<script setup lang="ts">
// Texte sur plusieurs lignes.
import { Textarea } from '@/shared/ui/textarea'

import type { FieldControlProps } from './types'

defineProps<FieldControlProps>()
const emit = defineEmits<{ 'update:modelValue': [value: unknown] }>()
</script>

<template>
  <Textarea
    :id="id"
    :model-value="(modelValue as string) ?? ''"
    :disabled="disabled"
    :maxlength="field.max_length ?? undefined"
    rows="6"
    class="font-mono text-xs"
    @update:model-value="emit('update:modelValue', $event)"
  />
</template>
