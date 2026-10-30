<script setup lang="ts">
import { type HTMLAttributes, provide, toRef } from 'vue'

import { cn } from '@/shared/lib/utils'

import { TREE_CONTEXT, type TreeItemKind } from '.'

const props = withDefaults(defineProps<{ class?: HTMLAttributes['class']; selected?: string; indicator?: boolean }>(), {
  selected: undefined,
  indicator: true,
})
const emit = defineEmits<{ select: [id: string, kind: TreeItemKind] }>()
const expanded = defineModel<string[]>('expanded', { default: () => [] })

provide(TREE_CONTEXT, {
  selected: toRef(props, 'selected'),
  expanded,
  indicator: props.indicator,
  toggle: (id) => {
    expanded.value = expanded.value.includes(id) ? expanded.value.filter((item) => item !== id) : [...expanded.value, id]
  },
  select: (id, kind) => emit('select', id, kind),
})
</script>

<template>
  <div role="tree" data-slot="file-tree" :class="cn('flex size-full flex-col gap-0.5 overflow-auto px-2', props.class)">
    <slot />
  </div>
</template>
