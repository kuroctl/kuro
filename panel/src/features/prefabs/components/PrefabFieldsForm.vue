<script setup lang="ts">
// Champs d'un groupe de réglages, générés depuis le prefab. Le formulaire (vee-validate) est
// fourni par le parent via `usePrefabForm` : ce composant ne fait que l'afficher.
import type { PrefabLocale } from '../composables/usePrefabLocale'
import type { FieldEntry } from '../composables/usePrefabSettings'
import type { Field } from '../manifest'
import PrefabField from './PrefabField.vue'

defineProps<{
  fields: FieldEntry[]
  locale: PrefabLocale
  locked?: (field: Field) => boolean
}>()
</script>

<template>
  <div class="grid gap-6">
    <PrefabField
      v-for="entry in fields"
      :key="entry.key"
      :name="entry.key"
      :field="entry.field"
      :locale="locale"
      :disabled="locked?.(entry.field)"
    />
  </div>
</template>
