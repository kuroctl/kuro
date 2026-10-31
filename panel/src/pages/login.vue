<script setup lang="ts">
import { useRoute, useRouter } from 'vue-router'

import { meQuery } from '@/features/account/account.queries'
import LoginFlow from '@/features/auth/components/LoginFlow.vue'
import { useSetupStatus } from '@/features/setup/setup.queries'
import LocaleSwitch from '@/shared/layout/LocaleSwitch.vue'
import { queryClient } from '@/shared/query/client'

const route = useRoute()
const router = useRouter()
const { data: setup } = useSetupStatus()

async function onAuthenticated() {
  await queryClient.prefetchQuery(meQuery)
  const redirect = typeof route.query.redirect === 'string' ? route.query.redirect : '/'
  await router.replace(redirect.startsWith('/') ? redirect : '/')
}
</script>

<template>
  <div
    class="relative flex min-h-dvh items-center justify-center overflow-hidden bg-rail p-4"
  >
    <!-- Halo d'ambiance, purement décoratif. -->
    <div
      class="pointer-events-none absolute -top-40 left-1/2 size-[640px] -translate-x-1/2 rounded-full bg-primary/20 blur-3xl"
      aria-hidden="true"
    />
    <LocaleSwitch class="absolute top-4 right-4" />
    <div class="relative flex w-full max-w-sm flex-col items-center gap-5">
      <p v-if="setup?.project_name" class="text-lg font-bold tracking-tight">{{ setup.project_name }}</p>
      <LoginFlow class="w-full" @authenticated="onAuthenticated" />
    </div>
  </div>
</template>
