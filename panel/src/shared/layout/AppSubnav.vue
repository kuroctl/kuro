<script setup lang="ts">
// Panneau de la section courante (serveurs, utilisateurs, paramètres, compte).
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import { usePermissions } from '@/features/account/account.queries'

import type { NavEntry } from './nav'
import NavPanel from './NavPanel.vue'
import { type SubnavItem, sectionOf } from './sections'

const { t } = useI18n()
const route = useRoute()
const section = computed(() => sectionOf(route.path))
const { can } = usePermissions()

/** Entrée active : la page exacte, ou l'une de ses sous-pages. La racine de la section ne garde
 * que les sous-pages qu'aucune autre entrée ne revendique (`/users/<id>`, pas `/users/roles`). */
function isActive(item: SubnavItem): boolean {
  if (!item.to) return false
  if (route.path === item.to) return true
  const below = (to?: string) => !!to && route.path.startsWith(`${to}/`)
  if (item.to !== section.value?.to) return below(item.to)
  return below(item.to) && !section.value.items.some((other) => other !== item && (other.to === route.path || (other.to !== item.to && below(other.to))))
}

const entries = computed<NavEntry[]>(() =>
  (section.value?.items ?? [])
    .filter((item) => !item.permission || can(item.permission))
    .map((item) => ({
      key: item.label,
      label: t(item.label),
      icon: item.icon,
      to: item.to,
      soon: item.soon,
      active: isActive(item),
    })),
)
</script>

<template>
  <NavPanel :entries="entries">
    <template #header>
      <h2 v-if="section" class="truncate text-lg font-bold">{{ t(section.title) }}</h2>
    </template>
  </NavPanel>
</template>
