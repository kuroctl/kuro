<script setup lang="ts">
import type { TabsListProps } from "reka-ui"
import type { HTMLAttributes } from "vue"
import { reactiveOmit } from "@vueuse/core"
import { cn } from "@/shared/lib/utils"
import { TabsIndicator, TabsList } from "reka-ui"

// `shrink-0` : `overflow-x-auto` ramène la hauteur minimale à 0 ; sans lui, un contenu plus haut
// que le conteneur `Tabs` (flex en colonne) écrase la barre d'onglets.
const props = defineProps<TabsListProps & { class?: HTMLAttributes["class"] }>()

const delegatedProps = reactiveOmit(props, "class")
</script>

<template>
  <TabsList
    data-slot="tabs-list"
    v-bind="delegatedProps"
    :class="cn(
      'bg-card text-muted-foreground relative inline-flex w-fit max-w-full shrink-0 items-center gap-1 overflow-x-auto rounded-xl border p-1.5 shadow-card',
      props.class,
    )"
  >
    <slot />
    <!-- Pastille de l'onglet actif : reka-ui fournit sa position et sa largeur. -->
    <TabsIndicator
      data-slot="tabs-indicator"
      class="absolute inset-y-1.5 left-0 w-(--reka-tabs-indicator-size) translate-x-(--reka-tabs-indicator-position) rounded-[calc(var(--radius)-3px)] bg-accent transition-[width,translate] duration-300 ease-out motion-reduce:transition-none"
    />
  </TabsList>
</template>
