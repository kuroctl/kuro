<script setup lang="ts">
// En-tête de page du design system : bouton du menu latéral, grand titre, actions à droite,
// détails facultatifs dessous. Hors de la coque (déconnexion en cours), pas de bouton de menu.
import { PanelLeft } from '@lucide/vue'
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'
import { useSidebar } from '@/shared/ui/sidebar'

defineProps<{ title: string }>()
const { t } = useI18n()
const sidebar = useSidebar(null)
</script>

<template>
  <header class="flex shrink-0 flex-wrap items-start justify-between gap-4 px-6 pt-6">
    <div class="flex min-w-0 items-start gap-2">
      <Button
        v-if="sidebar"
        variant="ghost"
        size="icon-sm"
        class="-ml-2 mt-0.5"
        :aria-label="t('nav.toggle')"
        @click="sidebar.toggleSidebar()"
      >
        <PanelLeft />
      </Button>
      <div class="grid min-w-0 gap-2">
        <h1 class="truncate text-[26px] leading-tight font-extrabold tracking-tight">{{ title }}</h1>
        <div v-if="$slots.meta" class="flex flex-wrap items-center gap-4 text-[13px] text-muted-foreground"><slot name="meta" /></div>
      </div>
    </div>
    <div v-if="$slots.default" class="flex items-center gap-2"><slot /></div>
  </header>
</template>
