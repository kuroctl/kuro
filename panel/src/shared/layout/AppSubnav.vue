<script setup lang="ts">
// Panneau de la section courante (serveurs, utilisateurs, paramètres, compte).
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import { usePermissions } from '@/features/account/account.queries'

import NavPanel from './NavPanel.vue'
import { sectionOf } from './sections'
import { subnavEntries } from './subnav'

const { t } = useI18n()
const route = useRoute()
const section = computed(() => sectionOf(route.path))
const { can } = usePermissions()

const entries = computed(() => subnavEntries(section.value, route.path, can, t))
</script>

<template>
  <NavPanel :entries="entries">
    <template #header>
      <h2 v-if="section" class="truncate text-lg font-bold">{{ t(section.title) }}</h2>
    </template>
  </NavPanel>
</template>
