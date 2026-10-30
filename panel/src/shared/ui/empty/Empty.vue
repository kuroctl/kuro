<script setup lang="ts">
import type { Component, HTMLAttributes } from "vue"
import { cn } from "@/shared/lib/utils"

// État vide du design system : icône dans un carré muted, titre, aide et action facultatifs.
const props = defineProps<{
  icon?: Component
  title?: string
  class?: HTMLAttributes["class"]
}>()
</script>

<template>
  <div data-slot="empty" :class="cn('flex flex-col items-center gap-2.5 rounded-xl border bg-card px-5 py-8 text-center shadow-card', props.class)">
    <span v-if="icon" class="grid size-12 place-items-center rounded-[14px] bg-muted text-muted-foreground">
      <component :is="icon" class="size-5" />
    </span>
    <p v-if="title" class="font-bold">{{ title }}</p>
    <div class="max-w-sm text-sm text-muted-foreground"><slot /></div>
    <slot name="action" />
  </div>
</template>
