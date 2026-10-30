<script lang="ts" setup>
import type { ToasterProps } from "vue-sonner"
import { CircleCheckIcon, InfoIcon, Loader2Icon, OctagonXIcon, TriangleAlertIcon, XIcon } from "@lucide/vue"
import { cn } from "@/shared/lib/utils"
import { Toaster as Sonner } from "vue-sonner"

const props = defineProps<ToasterProps>()
</script>

<template>
  <Sonner
    :class="cn('toaster group', props.class)"
    :style="{
      '--normal-bg': 'var(--popover)',
      '--normal-text': 'var(--popover-foreground)',
      '--normal-border': 'var(--border)',
      '--border-radius': 'var(--radius)',
    }"
    v-bind="props"
    :toast-options="{ class: 'shadow-card! rounded-xl! font-sans! [&_[data-icon]]:size-7! [&_[data-icon]]:mr-1!', ...props.toastOptions }"
  >
    <template #success-icon>
      <span class="grid size-7 place-items-center rounded-lg bg-success/16 text-success"><CircleCheckIcon class="size-4" /></span>
    </template>
    <template #info-icon>
      <span class="grid size-7 place-items-center rounded-lg bg-info/16 text-info"><InfoIcon class="size-4" /></span>
    </template>
    <template #warning-icon>
      <span class="grid size-7 place-items-center rounded-lg bg-warning/18 text-warning"><TriangleAlertIcon class="size-4" /></span>
    </template>
    <template #error-icon>
      <span class="grid size-7 place-items-center rounded-lg bg-destructive/16 text-destructive"><OctagonXIcon class="size-4" /></span>
    </template>
    <template #loading-icon>
      <div>
        <Loader2Icon class="size-4 animate-spin" />
      </div>
    </template>
    <template #close-icon>
      <XIcon class="size-4" />
    </template>
  </Sonner>
</template>
