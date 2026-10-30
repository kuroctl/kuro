<script setup lang="ts">
// Valeurs saisies pour un contenu (ADR-0030), champs déclarés par le prefab : une ligne par
// valeur pour une liste. Libellés et aide viennent des traductions du prefab.
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Textarea } from '@/shared/ui/textarea'

import type { ContentSlot } from '../content.api'

const props = defineProps<{
  fields: ContentSlot['fields']
  label: (field: string) => string
  help: (field: string) => string | undefined
  /** Erreurs de l'API, par `values.<champ>`. */
  errors?: Record<string, string>
}>()
const values = defineModel<Record<string, string>>({ required: true })

const id = (field: string) => `content-value-${field}`
const error = (field: string) => props.errors?.[`values.${field}`]
</script>

<template>
  <div class="grid gap-3">
    <div v-for="field in fields" :key="field.key" class="grid gap-1.5">
      <Label :for="id(field.key)">{{ label(field.key) }}</Label>
      <Textarea
        v-if="field.type === 'list'"
        :id="id(field.key)"
        v-model="values[field.key]"
        rows="3"
        class="font-mono text-sm"
        spellcheck="false"
        :aria-invalid="!!error(field.key)"
      />
      <Input
        v-else
        :id="id(field.key)"
        v-model="values[field.key]"
        spellcheck="false"
        :aria-invalid="!!error(field.key)"
      />
      <p v-if="error(field.key)" class="text-sm text-destructive">{{ error(field.key) }}</p>
      <p v-else-if="help(field.key)" class="text-xs text-muted-foreground">{{ help(field.key) }}</p>
    </div>
  </div>
</template>
