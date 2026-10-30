<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'

import { useSessionStore } from '@/features/auth/session.store'
import AppShell from '@/shared/layout/AppShell.vue'
import { BARE_PATHS, PUBLIC_PATHS } from '@/shared/router'
import { useTheme } from '@/shared/theme/useTheme'
import { Toaster } from '@/shared/ui/sonner'
import { TooltipProvider } from '@/shared/ui/tooltip'

const route = useRoute()
const theme = useTheme()
const session = useSessionStore()
// Coque seulement une fois la session ouverte : pendant la navigation initiale, la route vaut
// encore `/` et la coque lancerait des requêtes sans jeton.
const inShell = computed(
  () => session.authenticated && !PUBLIC_PATHS.includes(route.path) && !BARE_PATHS.includes(route.path),
)
</script>

<template>
  <TooltipProvider :delay-duration="200">
    <AppShell v-if="inShell">
      <RouterView v-slot="{ Component }">
        <Transition name="fade" mode="out-in">
          <component :is="Component" :key="route.path" />
        </Transition>
      </RouterView>
    </AppShell>
    <RouterView v-else />
  </TooltipProvider>
  <Toaster rich-colors position="bottom-right" :theme="theme === 'light' ? 'light' : 'dark'" />
</template>
