<script setup lang="ts">
// Arbre du menu latéral : pages à icône, groupes repliables (+ / −) dont les pages sont reliées
// par un trait. Les groupes sont ouverts par défaut ; une recherche les ouvre tous, et le groupe
// de la page active se rouvre quand on y arrive.
import { Minus, Plus } from '@lucide/vue'
import { reactive, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { Badge } from '@/shared/ui/badge'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/shared/ui/collapsible'
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
} from '@/shared/ui/sidebar'

import type { NavEntry } from './nav'

const props = defineProps<{ entries: NavEntry[]; searching?: boolean }>()
const { t } = useI18n()

// Groupes repliés par l'utilisateur, gardés d'une page à l'autre.
const folded = reactive(new Set<string>())

function setOpen(key: string, open: boolean) {
  if (open) folded.delete(key)
  else folded.add(key)
}

watch(
  () => props.entries.filter((entry) => entry.children?.some((child) => child.active)).map((entry) => entry.key),
  (keys) => keys.forEach((key) => folded.delete(key)),
  { immediate: true },
)

// Traits des sous-pages (repris du « branched menu » de nxui) : un tronc, une branche courbe par
// page, et un trait d'accent qui part du haut du tronc et se trace jusqu'à la page active.
const ROW = 32 // hauteur d'une sous-page (h-8), sans espace entre elles pour que les traits tombent juste
const PAD = 4 // py-1 de la liste
const TRUNK = 0.75 // moitié de l'épaisseur du trait, pour qu'il ne soit pas rogné
const RADIUS = 8
const END = 12 // fin des branches, avant le texte

const rowY = (k: number) => PAD + k * ROW + ROW / 2
const trunkPath = (count: number) => `M ${TRUNK} 0 V ${rowY(count - 1) - RADIUS}`
const branchPath = (k: number, from = rowY(k) - RADIUS) =>
  `M ${TRUNK} ${from} V ${rowY(k) - RADIUS} A ${RADIUS} ${RADIUS} 0 0 0 ${TRUNK + RADIUS} ${rowY(k)} H ${END}`
const reachLength = (k: number) => rowY(k) - RADIUS + (Math.PI * RADIUS) / 2 + (END - TRUNK - RADIUS)

function reachStyle(k: number, active?: boolean) {
  const length = reachLength(k)
  return { strokeDasharray: `${length}`, strokeDashoffset: `${active ? 0 : length}` }
}
</script>

<template>
  <SidebarMenu class="gap-1">
    <template v-for="entry in entries" :key="entry.key">
      <Collapsible
        v-if="entry.children"
        as-child
        :open="searching || !folded.has(entry.key)"
        @update:open="(open: boolean) => setOpen(entry.key, open)"
      >
        <SidebarMenuItem>
          <CollapsibleTrigger as-child>
            <SidebarMenuButton class="group/trigger">
              <component :is="entry.icon" />
              <span>{{ entry.label }}</span>
              <Minus class="ml-auto text-muted-foreground group-data-[state=closed]/trigger:hidden" />
              <Plus class="ml-auto text-muted-foreground group-data-[state=open]/trigger:hidden" />
            </SidebarMenuButton>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <ul class="relative ml-[17px] flex flex-col py-1">
              <svg
                class="pointer-events-none absolute top-0 left-0 overflow-visible fill-none stroke-[1.5] [stroke-linecap:round]"
                :width="END"
                :height="rowY(entry.children.length - 1) + ROW / 2 + PAD"
                aria-hidden="true"
              >
                <path class="stroke-muted-foreground/35" :d="trunkPath(entry.children.length)" />
                <path
                  v-for="(child, k) in entry.children"
                  :key="`branch-${child.key}`"
                  class="stroke-muted-foreground/35"
                  :d="branchPath(k)"
                />
                <path
                  v-for="(child, k) in entry.children"
                  :key="`reach-${child.key}`"
                  class="stroke-foreground transition-[stroke-dashoffset] duration-400 ease-out motion-reduce:transition-none"
                  :d="branchPath(k, 0)"
                  :style="reachStyle(k, child.active)"
                />
              </svg>
              <SidebarMenuSubItem v-for="child in entry.children" :key="child.key" class="pl-4">
                <SidebarMenuSubButton as-child :is-active="child.active" class="h-8 translate-x-0 px-2.5 text-muted-foreground data-[active=true]:font-semibold">
                  <RouterLink :to="child.to ?? ''"><span>{{ child.label }}</span></RouterLink>
                </SidebarMenuSubButton>
              </SidebarMenuSubItem>
            </ul>
          </CollapsibleContent>
        </SidebarMenuItem>
      </Collapsible>

      <SidebarMenuItem v-else>
        <SidebarMenuButton v-if="entry.to && !entry.soon" as-child :is-active="entry.active">
          <RouterLink :to="entry.to">
            <component :is="entry.icon" />
            <span>{{ entry.label }}</span>
          </RouterLink>
        </SidebarMenuButton>
        <SidebarMenuButton v-else aria-disabled="true" class="cursor-not-allowed opacity-60">
          <component :is="entry.icon" />
          <span>{{ entry.label }}</span>
          <Badge variant="secondary" class="ml-auto h-5 px-2 text-[10px]">{{ t('common.soon') }}</Badge>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </template>
  </SidebarMenu>
</template>
