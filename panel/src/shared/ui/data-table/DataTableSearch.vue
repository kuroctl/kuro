<script setup lang="ts">
// Recherche d'une liste, appliquée après une courte pause de frappe.
import { Search } from '@lucide/vue'
import { useDebounceFn } from '@vueuse/core'
import { ref, watch } from 'vue'

import { Input } from '@/shared/ui/input'

defineProps<{ placeholder: string }>()
const query = defineModel<string>({ required: true })
const draft = ref(query.value)

watch(query, (value) => {
  if (value !== draft.value.trim()) draft.value = value
})
const apply = useDebounceFn((value: string) => {
  query.value = value
}, 250)
</script>

<template>
  <div class="relative w-full max-w-xs">
    <Search class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
    <Input v-model="draft" type="search" :placeholder="placeholder" :aria-label="placeholder" class="h-8 pl-8" @update:model-value="(v) => apply(String(v))" />
  </div>
</template>
