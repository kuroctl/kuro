<script setup lang="ts">
import { File } from '@lucide/vue'
import { computed, inject } from 'vue'

import { cn } from '@/shared/lib/utils'

import { TREE_CONTEXT, type TreeItemProps } from '.'

const props = withDefaults(defineProps<TreeItemProps>(), { class: undefined, isSelectable: true })

const tree = inject(TREE_CONTEXT)
if (!tree) throw new Error('<File> must be used inside <Tree>')

const isSelected = computed(() => tree.selected.value === props.id)
</script>

<template>
  <button
    type="button"
    role="treeitem"
    :aria-selected="isSelected"
    :disabled="!isSelectable"
    :title="name"
    :class="
      cn(
        'flex w-full min-w-0 items-center gap-1.5 rounded-md py-1 pr-1.5 pl-6 text-left text-sm text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none',
        isSelected && 'bg-muted font-medium text-foreground',
        !isSelectable && 'cursor-not-allowed opacity-50',
        props.class,
      )
    "
    @click="isSelectable && tree!.select(id, 'file')"
  >
    <slot name="icon"><File class="size-4 shrink-0" /></slot>
    <span class="truncate select-none">{{ name }}</span>
  </button>
</template>
