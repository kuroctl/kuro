<script setup lang="ts">
// Filtre à facettes : valeurs cochables avec leur nombre de résultats (les autres filtres
// appliqués). Plusieurs valeurs : l'une d'elles.
import { CirclePlus } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Checkbox } from '@/shared/ui/checkbox'
import { Popover, PopoverContent, PopoverTrigger } from '@/shared/ui/popover'
import { Separator } from '@/shared/ui/separator'

export interface FacetOption {
  value: string
  label: string
  count?: number
}

const props = defineProps<{ title: string; options: FacetOption[] }>()
const selected = defineModel<string[]>({ required: true })
const { t } = useI18n()

const labels = computed(() => props.options.filter((option) => selected.value.includes(option.value)).map((o) => o.label))

function toggle(value: string, on: boolean) {
  selected.value = on ? [...selected.value, value] : selected.value.filter((v) => v !== value)
}
</script>

<template>
  <Popover>
    <PopoverTrigger as-child>
      <Button variant="outline" size="sm" class="h-8 border-dashed">
        <CirclePlus class="size-4" />
        {{ title }}
        <template v-if="selected.length">
          <Separator orientation="vertical" class="mx-1 h-4" />
          <Badge v-if="selected.length > 2" variant="secondary" class="rounded-sm px-1 font-normal">
            {{ t('dataTable.selectedCount', { count: selected.length }) }}
          </Badge>
          <Badge v-for="label in labels" v-else :key="label" variant="secondary" class="rounded-sm px-1 font-normal">{{ label }}</Badge>
        </template>
      </Button>
    </PopoverTrigger>
    <PopoverContent class="w-60 p-1" align="start">
      <p v-if="!options.length" class="px-2 py-3 text-center text-sm text-muted-foreground">{{ t('dataTable.noValue') }}</p>
      <div class="max-h-72 overflow-y-auto">
        <label
          v-for="option in options"
          :key="option.value"
          class="flex cursor-pointer items-center gap-2 rounded-sm px-2 py-1.5 text-sm hover:bg-accent"
        >
          <Checkbox :model-value="selected.includes(option.value)" @update:model-value="(on) => toggle(option.value, on === true)" />
          <span class="min-w-0 flex-1 truncate">{{ option.label }}</span>
          <span v-if="option.count !== undefined" class="font-mono text-xs text-muted-foreground">{{ option.count }}</span>
        </label>
      </div>
      <template v-if="selected.length">
        <Separator class="my-1" />
        <Button variant="ghost" size="sm" class="w-full" @click="selected = []">{{ t('dataTable.clearFilter') }}</Button>
      </template>
    </PopoverContent>
  </Popover>
</template>
