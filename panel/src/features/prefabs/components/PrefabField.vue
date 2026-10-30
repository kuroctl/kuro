<script setup lang="ts">
// Un champ de prefab dans un formulaire vee-validate : libellé, aide et options traduits par le
// prefab, composant de saisie choisi par le registre selon le type.
import { RotateCw } from '@lucide/vue'
import { useI18n } from 'vue-i18n'

import { Badge } from '@/shared/ui/badge'
import { FormControl, FormDescription, FormField, FormItem, FormLabel, FormMessage } from '@/shared/ui/form'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/shared/ui/tooltip'

import type { PrefabLocale } from '../composables/usePrefabLocale'
import { fieldControls } from '../fields/registry'
import type { Field } from '../manifest'

const props = defineProps<{
  name: string
  field: Field
  locale: PrefabLocale
  disabled?: boolean
}>()
const { t } = useI18n()
</script>

<template>
  <FormField v-slot="{ value, handleChange }" :name="name">
    <FormItem>
      <div class="flex items-center gap-2">
        <FormLabel>{{ locale.fieldLabel(name) }}</FormLabel>
        <Tooltip v-if="field.restart">
          <TooltipTrigger as-child>
            <Badge variant="outline" class="gap-1 text-[10px]">
              <RotateCw class="size-3" />
              {{ t('prefabs.field.restart') }}
            </Badge>
          </TooltipTrigger>
          <TooltipContent>{{ t('prefabs.field.restartHelp') }}</TooltipContent>
        </Tooltip>
        <Badge v-if="disabled" variant="secondary" class="text-[10px]">{{ t('prefabs.field.locked') }}</Badge>
      </div>
      <FormControl>
        <component
          :is="fieldControls[field.type]"
          :field="field"
          :model-value="value"
          :disabled="disabled"
          :option-label="(option: unknown) => props.locale.optionLabel(name, option)"
          @update:model-value="handleChange"
        />
      </FormControl>
      <FormDescription v-if="locale.fieldHelp(name)">{{ locale.fieldHelp(name) }}</FormDescription>
      <FormMessage />
    </FormItem>
  </FormField>
</template>
