<script setup lang="ts">
// Texte sur une ligne, mot de passe ou secret.
import { useI18n } from 'vue-i18n'

import { Input } from '@/shared/ui/input'

import type { FieldControlProps } from './types'

const props = defineProps<FieldControlProps>()
const emit = defineEmits<{ 'update:modelValue': [value: unknown] }>()
const { t } = useI18n()

const hidden = props.field.type === 'password' || props.field.type === 'secret'
</script>

<template>
  <Input
    :id="id"
    :type="hidden ? 'password' : 'text'"
    :model-value="(modelValue as string) ?? ''"
    :disabled="disabled"
    :maxlength="field.max_length ?? undefined"
    :autocomplete="hidden ? 'new-password' : 'off'"
    :placeholder="field.type === 'secret' ? t('prefabs.field.secretPlaceholder') : undefined"
    @update:model-value="emit('update:modelValue', $event)"
  />
</template>
