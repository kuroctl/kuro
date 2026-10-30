<script setup lang="ts">
// Panneau d'un serveur : ses pages, puis les groupes de réglages du prefab (sections générale et
// avancée) et le contenu (mods, plugins) qu'il déclare, chacun en groupe repliable.
import { Activity, Archive, ArrowLeft, Bell, CalendarClock, Folder, Gauge, LayoutDashboard, Package, Settings2, SlidersHorizontal, Terminal } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import { usePermissions } from '@/features/account/account.queries'
import { useContentSlots } from '@/features/content/content.queries'
import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { usePrefabSettings } from '@/features/prefabs/composables/usePrefabSettings'
import { usePrefab } from '@/features/prefabs/prefabs.queries'
import type { NavEntry } from '@/shared/layout/nav'
import NavPanel from '@/shared/layout/NavPanel.vue'
import { Badge } from '@/shared/ui/badge'
import { Skeleton } from '@/shared/ui/skeleton'

import { useServer } from '../servers.queries'
import ServerStateBadge from './ServerStateBadge.vue'

const { t, locale: language } = useI18n()
const route = useRoute()
const id = computed(() => String((route.params as Record<string, unknown>).id ?? ''))
const { data: server } = useServer(id)
const { data: prefab } = usePrefab(
  () => server.value?.prefab.id,
  () => server.value?.prefab.version,
)
const { sections } = usePrefabSettings(() => prefab.value?.manifest)
const locale = usePrefabLocale(() => server.value?.prefab)
const { can } = usePermissions()
const { data: slots } = useContentSlots(id, () => can('servers.content.read'))

const SECTION_ICONS: Record<string, NavEntry['icon']> = { general: SlidersHorizontal, advanced: Settings2 }

const entries = computed<NavEntry[]>(() => {
  const base = `/servers/${id.value}`
  // Les fichiers gardent leur entrée active dans les sous-dossiers.
  const page = (key: string, icon: NavEntry['icon'], to: string, prefix = false): NavEntry => ({
    key,
    label: t(`servers.nav.${key}`),
    icon,
    to,
    active: route.path === to || (prefix && route.path.startsWith(`${to}/`)),
  })
  const list: NavEntry[] = [
    page('overview', LayoutDashboard, base),
    page('console', Terminal, `${base}/console`),
    page('stats', Activity, `${base}/stats`),
    page('files', Folder, `${base}/files`, true),
    page('backups', Archive, `${base}/backups`),
    page('schedules', CalendarClock, `${base}/schedules`, true),
    page('notifications', Bell, `${base}/notifications`),
    page('resources', Gauge, `${base}/resources`),
  ]
  for (const section of sections.value) {
    list.push({
      key: section.key,
      label: t(`servers.nav.${section.key}`),
      icon: SECTION_ICONS[section.key] ?? SlidersHorizontal,
      children: section.groups.map((group) => {
        const to = `${base}/settings/${section.key}/${group.key}`
        return { key: `${section.key}.${group.key}`, label: locale.groupTitle(group.key), icon: SlidersHorizontal, to, active: route.path === to }
      }),
    })
  }
  if (slots.value?.length) {
    list.push({
      key: 'content',
      label: t('servers.nav.content'),
      icon: Package,
      children: slots.value.map((slot) => {
        const to = `${base}/content/${slot.slot}`
        return { key: `content.${slot.slot}`, label: locale.contentTitle(slot.slot), icon: Package, to, active: route.path === to }
      }),
    })
  }
  return list
})
</script>

<template>
  <NavPanel :entries="server ? entries : []">
    <template #header>
      <RouterLink to="/servers/list" class="inline-flex w-fit items-center gap-1.5 text-[13px] font-semibold text-muted-foreground hover:text-foreground">
        <ArrowLeft class="size-4" />{{ t('servers.backToList') }}
      </RouterLink>
      <div v-if="server" class="grid gap-2">
        <h2 class="truncate text-lg font-bold">{{ server.name }}</h2>
        <div class="flex flex-wrap gap-1.5">
          <ServerStateBadge :state="server.state" :suspended="!!server.suspended_at" />
          <Badge v-if="server.trial" variant="info" :title="t('servers.trial.help')">
            {{ t('servers.trial.badge', { time: new Date(server.trial.expires_at).toLocaleTimeString(language, { timeStyle: 'short' }) }) }}
          </Badge>
        </div>
      </div>
      <Skeleton v-else class="h-7 w-40" />
    </template>
    <template v-if="!server" #before>
      <div class="grid gap-2 px-1"><Skeleton v-for="index in 5" :key="index" class="h-9" /></div>
    </template>
  </NavPanel>
</template>
