<script setup lang="ts">
// Coque du panel (Sidebar de shadcn-vue) : un rail d'icônes et le panneau de la section ou du
// serveur courant, repliable sur le rail seul, en carte flottante ; le contenu ; un panneau droit
// ponctuel.
import { useRoute } from 'vue-router'

import ServerSubnav from '@/features/servers/components/ServerSubnav.vue'
import { Sidebar, SidebarInset, SidebarProvider } from '@/shared/ui/sidebar'

import AppRail from './AppRail.vue'
import AppSubnav from './AppSubnav.vue'

const route = useRoute()

/** Panneau selon la variante : barre latérale (sidebar), carte (floating), fondu dans le fond
 * de la page, le contenu faisant carte (inset). */
const panel = [
  'flex-none w-[calc(var(--sidebar-width)-var(--sidebar-width-icon))] bg-sidebar',
  'group-data-[variant=floating]:w-[calc(var(--sidebar-width)-var(--sidebar-width-icon)-(--spacing(4))-2px)] group-data-[variant=floating]:bg-card',
  // Inset : seule la marge gauche est retirée, le panneau remplit le reste jusqu'au contenu.
  'group-data-[variant=inset]:w-[calc(var(--sidebar-width)-var(--sidebar-width-icon)-(--spacing(2)))] group-data-[variant=inset]:bg-transparent',
]
</script>

<template>
  <SidebarProvider class="h-dvh overflow-hidden" :style="{ '--sidebar-width': '20rem', '--sidebar-width-icon': '4.5rem' }">
    <Sidebar variant="inset" collapsible="icon" class="*:data-[sidebar=sidebar]:flex-row  pr-0!">
      <AppRail />
      <!-- Largeur fixe (moins la marge et la bordure des variantes floating et inset) : en se
           repliant, le menu rogne le panneau au lieu de faire passer ses libellés à la ligne. -->
      <Sidebar collapsible="none" :class="panel" class="border-y ">
        <!-- Une page peut demander le panneau du serveur affiché : `definePage({ meta: { subnav: 'server' } })`. -->
        <ServerSubnav v-if="route.meta.subnav === 'server'" />
        <AppSubnav v-else />
      </Sidebar>
    </Sidebar>
    <SidebarInset class="min-w-0 overflow-y-auto">
      <slot />
    </SidebarInset>
    <!-- Panneau droit contextuel (détail d'un nœud, d'un événement…) : tranche P2. -->
    <slot name="aside" />
  </SidebarProvider>
</template>
