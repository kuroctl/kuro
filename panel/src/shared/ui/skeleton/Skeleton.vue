<script setup lang="ts">
import type { HTMLAttributes } from "vue"
import { cn } from "@/shared/lib/utils"

interface SkeletonProps {
  class?: HTMLAttributes["class"]
}

const props = defineProps<SkeletonProps>()
</script>

<template>
  <div
    data-slot="skeleton"
    :class="cn('animate-shimmer rounded-md bg-[linear-gradient(90deg,var(--muted),color-mix(in_oklch,var(--muted)_60%,var(--card)),var(--muted))] bg-size-[200%_100%] motion-reduce:animate-none', props.class)"
  />
</template>
