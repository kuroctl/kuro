<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/shared/lib/utils"

const props = defineProps<{
  class?: HTMLAttributes["class"]
}>()
</script>

<template>
  <div data-slot="alert-description" :class="cn('text-[13px] text-muted-foreground', props.class)">
    <slot />
  </div>
</template>
