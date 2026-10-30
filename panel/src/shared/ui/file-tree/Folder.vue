<script setup lang="ts">
import { ChevronRight, Folder, FolderOpen } from '@lucide/vue'
import { computed, inject } from 'vue'

import { cn } from '@/shared/lib/utils'

import { TREE_CONTEXT, type TreeItemProps } from '.'
import TreeIndicator from './TreeIndicator.vue'

const props = withDefaults(defineProps<TreeItemProps>(), { class: undefined, isSelectable: true })

const tree = inject(TREE_CONTEXT)
if (!tree) throw new Error('<Folder> must be used inside <Tree>')

const isExpanded = computed(() => tree.expanded.value.includes(props.id))
const isSelected = computed(() => tree.selected.value === props.id)

function onClick() {
  if (!props.isSelectable) return
  // Ouvrir un dossier refermé le sélectionne ; recliquer sur le dossier sélectionné le replie.
  if (!isExpanded.value || isSelected.value) tree!.toggle(props.id)
  tree!.select(props.id, 'folder')
}
</script>

<template>
  <div role="treeitem" :aria-label="name" :aria-expanded="isExpanded" :aria-selected="isSelected" class="relative">
    <button
      type="button"
      :disabled="!isSelectable"
      :title="name"
      :class="
        cn(
          'flex w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1 text-left text-sm transition-colors hover:bg-muted/70 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none',
          isSelected && 'bg-muted font-medium text-foreground',
          !isSelectable && 'cursor-not-allowed opacity-50',
          props.class,
        )
      "
      @click="onClick"
    >
      <ChevronRight class="size-3.5 shrink-0 text-muted-foreground transition-transform" :class="{ 'rotate-90': isExpanded }" />
      <FolderOpen v-if="isExpanded" class="size-4 shrink-0 text-primary" />
      <Folder v-else class="size-4 shrink-0 text-primary" />
      <span class="truncate select-none">{{ name }}</span>
    </button>
    <div v-if="isExpanded" role="group" class="relative">
      <TreeIndicator v-if="tree.indicator" aria-hidden="true" />
      <div class="ml-4 flex flex-col gap-0.5 py-0.5">
        <slot />
      </div>
    </div>
  </div>
</template>
