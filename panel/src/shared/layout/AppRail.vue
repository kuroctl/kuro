<script setup lang="ts">
// Rail de gauche : la marque, une icône par section, le compte connecté en bas. Sur mobile, le
// menu (tiroir) se referme dès qu'une page est choisie.
import { computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { useSetupStatus } from '@/features/setup/setup.queries'
import { cn } from '@/shared/lib/utils'
import { useRoute } from 'vue-router'

import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from '@/shared/ui/sidebar'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/shared/ui/tooltip'
import AccountMenu from './AccountMenu.vue'
import { sectionOf, sections } from './sections'

const { t } = useI18n()
// Nom du projet (ADR-0045), « Kuro » tant qu'il n'est pas réglé.
const { data: setup } = useSetupStatus()
const projectName = computed(() => setup.value?.project_name || t('app.name'))

const route = useRoute()
const current = computed(() => sectionOf(route.path)?.key)
const railSections = sections.filter((section) => section.icon)

const { setOpenMobile } = useSidebar()
watch(
  () => route.path,
  () => setOpenMobile(false),
)
</script>

<template>
  <Sidebar collapsible="none" 
    :class="cn(
      'w-(--sidebar-width-icon)! shrink-0 border-r border-sidebar-border bg-rail group-data-[collapsible=icon]:w-full! group-data-[variant=floating]:bg-sidebar',
      'group-data-[variant=inset]:rounded-l-xl group-data-[variant=inset]:border-y group-data-[variant=inset]:border-l group-data-[variant=inset]:border-r-0! group-data-[variant=inset]:pr-0 group-data-[variant=inset]:bg-card group-data-[collapsible=icon]:border-none! group-data-[collapsible=icon]:pr-0!')"
    >
    <SidebarHeader class="items-center pt-5 pb-3">
      <RouterLink
        to="/servers"
        :aria-label="projectName"
        :title="projectName"
        class="flex size-10 items-center justify-center rounded-xl bg-foreground text-lg font-extrabold text-background"
      >
        {{ projectName.charAt(0).toUpperCase() }}
      </RouterLink>
    </SidebarHeader>
    <SidebarContent>
      <SidebarMenu class="items-center gap-1.5 py-2">
        <SidebarMenuItem v-for="section in railSections" :key="section.key">
          <Tooltip>
            <TooltipTrigger as-child>
              <SidebarMenuButton
                as-child
                :is-active="current === section.key"
                class="size-10 justify-center p-0 text-muted-foreground hover:bg-muted hover:text-foreground data-[active=true]:bg-sidebar-accent [&>svg]:size-5"
              >
                <RouterLink :to="section.to" :aria-label="t(section.label)">
                  <component :is="section.icon" />
                </RouterLink>
              </SidebarMenuButton>
            </TooltipTrigger>
            <TooltipContent side="right">{{ t(section.label) }}</TooltipContent>
          </Tooltip>
        </SidebarMenuItem>
      </SidebarMenu>
    </SidebarContent>
    <SidebarFooter class="items-center pb-5">
      <AccountMenu />
    </SidebarFooter>
  </Sidebar>
</template>
