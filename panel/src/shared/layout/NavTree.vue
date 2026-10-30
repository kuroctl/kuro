<script setup lang="ts">
// Arbre du menu latéral : pages à icône, groupes repliables (+ / −) dont les pages sont reliées
// par un trait. Les groupes sont ouverts par défaut ; une recherche les ouvre tous.
import { Minus, Plus } from '@lucide/vue'
import { reactive } from 'vue'
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

defineProps<{ entries: NavEntry[]; searching?: boolean }>()
const { t } = useI18n()

// Groupes repliés par l'utilisateur, gardés d'une page à l'autre.
const folded = reactive(new Set<string>())

function setOpen(key: string, open: boolean) {
  if (open) folded.delete(key)
  else folded.add(key)
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
            <ul class="ml-[17px] flex flex-col gap-0.5 py-1">
              <SidebarMenuSubItem
                v-for="child in entry.children"
                :key="child.key"
                class="relative pl-4 before:absolute before:top-0 before:left-0 before:h-1/2 before:w-3 before:rounded-bl-lg before:border-b before:border-l before:border-muted-foreground/35 after:absolute after:top-0 after:-bottom-0.5 after:left-0 after:border-l after:border-muted-foreground/35 last:after:hidden"
              >
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
