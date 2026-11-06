<script setup lang="ts">
// Ce qui empêche une action (suppression d'un nœud qui porte des serveurs…) : chaque objet en
// cause avec un lien vers l'endroit où le régler, et au besoin un lien vers la liste complète.
import { ChevronRight } from "@lucide/vue"
import type { RouteLocationRaw } from "vue-router"

import type { Blocker } from "./types"

defineProps<{ items: Blocker[], more?: { label: string, to: RouteLocationRaw } }>()
</script>

<template>
  <ul data-slot="blocked-by" class="max-h-60 divide-y overflow-y-auto rounded-lg border">
    <li v-for="item in items" :key="item.key">
      <RouterLink :to="item.to" class="flex items-center gap-2.5 px-3 py-2.5 text-sm transition-colors hover:bg-muted/50">
        <span class="min-w-0 flex-1 truncate font-medium">{{ item.label }}</span>
        <span v-if="item.detail" class="shrink-0 text-xs text-muted-foreground">{{ item.detail }}</span>
        <ChevronRight class="size-3.5 shrink-0 text-muted-foreground" />
      </RouterLink>
    </li>
    <li v-if="more">
      <RouterLink :to="more.to" class="flex items-center gap-2.5 px-3 py-2.5 text-sm text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground">
        <span class="flex-1">{{ more.label }}</span>
        <ChevronRight class="size-3.5 shrink-0" />
      </RouterLink>
    </li>
  </ul>
</template>
