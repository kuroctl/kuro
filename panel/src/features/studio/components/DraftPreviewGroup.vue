<script setup lang="ts">
// Un groupe de réglages du brouillon, rendu comme à la création d'un serveur : rien n'est envoyé.
import PrefabFieldsForm from '@/features/prefabs/components/PrefabFieldsForm.vue'
import type { PrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { usePrefabForm } from '@/features/prefabs/composables/usePrefabForm'
import type { FieldEntry } from '@/features/prefabs/composables/usePrefabSettings'

const props = defineProps<{ fields: FieldEntry[]; locale: PrefabLocale }>()

usePrefabForm(props.fields, {}, 'create')
</script>

<template>
  <PrefabFieldsForm :fields="fields" :locale="locale" />
</template>
