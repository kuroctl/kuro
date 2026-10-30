<script setup lang="ts">
import type { SelectTriggerProps } from "reka-ui"
import type { HTMLAttributes } from "vue"
import { ChevronDown } from "@lucide/vue"
import { reactiveOmit } from "@vueuse/core"
import { cn } from "@/shared/lib/utils"
import { SelectIcon, SelectTrigger, useForwardProps } from "reka-ui"

const props = withDefaults(
  defineProps<SelectTriggerProps & { class?: HTMLAttributes["class"], size?: "sm" | "default" }>(),
  { size: "default" },
)

const delegatedProps = reactiveOmit(props, "class", "size")
const forwardedProps = useForwardProps(delegatedProps)
</script>

<template>
  <SelectTrigger
    data-slot="select-trigger"
    :data-size="size"
    v-bind="forwardedProps"
    :class="cn(
      `border-input data-[placeholder]:text-muted-foreground [&_svg:not([class*='text-'])]:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/25 aria-invalid:ring-3 aria-invalid:ring-destructive/18 aria-invalid:border-destructive data-[state=open]:border-ring data-[state=open]:ring-3 data-[state=open]:ring-ring/25 hover:border-[color-mix(in_oklch,var(--input)_50%,var(--foreground))] flex w-fit items-center justify-between gap-2 rounded-lg border bg-muted px-3 py-2 text-sm whitespace-nowrap transition-[color,border-color,box-shadow] outline-none focus-visible:ring-3 disabled:cursor-not-allowed disabled:opacity-50 data-[size=default]:h-10 data-[size=sm]:h-8 *:data-[slot=select-value]:line-clamp-1 *:data-[slot=select-value]:flex *:data-[slot=select-value]:items-center *:data-[slot=select-value]:gap-2 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4`,
      props.class,
    )"
  >
    <slot />
    <SelectIcon as-child>
      <ChevronDown class="size-4 text-muted-foreground transition-transform in-data-[state=open]:rotate-180" />
    </SelectIcon>
  </SelectTrigger>
</template>
