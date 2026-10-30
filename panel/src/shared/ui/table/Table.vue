<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/shared/lib/utils"

const props = defineProps<{
  class?: HTMLAttributes["class"]
}>()
</script>

<template>
  <div data-slot="table-container" class="relative w-full overflow-auto rounded-xl border bg-card shadow-card">
    <table data-slot="table" :class="cn('w-full caption-bottom text-[13.5px]', props.class)">
      <slot />
    </table>
  </div>
</template>
