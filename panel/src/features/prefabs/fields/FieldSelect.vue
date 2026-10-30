<script setup lang="ts">
// Choix parmi les options du prefab, libellés traduits par le prefab.
import { computed } from 'vue'

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

import type { FieldControlProps } from './types'

const props = defineProps<FieldControlProps>()
const emit = defineEmits<{ 'update:modelValue': [value: unknown] }>()

// Les valeurs du Select sont des chaînes : on retrouve l'option d'origine (nombre ou texte).
const options = computed(() => (props.field.options ?? []).map((value) => ({ value, key: String(value) })))
const selected = computed(() => (props.modelValue === undefined || props.modelValue === null ? undefined : String(props.modelValue)))

function select(key: unknown) {
  emit('update:modelValue', options.value.find((option) => option.key === key)?.value)
}
</script>

<template>
  <Select :model-value="selected" :disabled="disabled" @update:model-value="select">
    <SelectTrigger :id="id" class="w-full">
      <!-- Libellé explicite : sinon le Select affiche la valeur brute tant qu'il n'a pas été ouvert. -->
      <SelectValue>{{ selected === undefined ? '' : optionLabel(modelValue) }}</SelectValue>
    </SelectTrigger>
    <SelectContent>
      <SelectItem v-for="option in options" :key="option.key" :value="option.key">
        {{ optionLabel(option.value) }}
      </SelectItem>
    </SelectContent>
  </Select>
</template>
