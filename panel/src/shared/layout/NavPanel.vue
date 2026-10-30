<script setup lang="ts">
// Panneau du menu latéral : en-tête fourni par la page (section ou serveur), recherche, arbre.
import { Search } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { SidebarContent, SidebarHeader, SidebarInput } from '@/shared/ui/sidebar'

import { filterEntries, type NavEntry } from './nav'
import NavTree from './NavTree.vue'

const props = defineProps<{ entries: NavEntry[] }>()
const { t } = useI18n()

const query = ref('')
const shown = computed(() => filterEntries(props.entries, query.value))
</script>

<template>
  <SidebarHeader class="gap-4 px-4 pt-5 pb-3">
    <slot name="header" />
    <div class="relative">
      <Search class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
      <SidebarInput v-model="query" type="search" :placeholder="t('nav.search')" :aria-label="t('nav.search')" class="h-10 bg-background pl-9" />
    </div>
  </SidebarHeader>
  <SidebarContent class="px-2 pb-4">
    <slot name="before" />
    <nav class="px-1">
      <NavTree :entries="shown" :searching="!!query.trim()" />
      <p v-if="query.trim() && shown.length === 0" class="px-2.5 py-2 text-sm text-muted-foreground">{{ t('nav.noMatch') }}</p>
    </nav>
  </SidebarContent>
</template>
