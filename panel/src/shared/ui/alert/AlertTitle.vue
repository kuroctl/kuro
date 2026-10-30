<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/shared/lib/utils"

const props = defineProps<{
  class?: HTMLAttributes["class"]
}>()
</script>

<template>
  <p data-slot="alert-title" :class="cn('font-bold text-foreground', props.class)">
    <slot />
  </p>
</template>
