<script setup lang="ts">
import { useRoute, useRouter } from 'vue-router'

import { meQuery } from '@/features/account/account.queries'
import LoginFlow from '@/features/auth/components/LoginFlow.vue'
import { queryClient } from '@/shared/query/client'

const route = useRoute()
const router = useRouter()

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
    <LoginFlow class="relative" @authenticated="onAuthenticated" />
  </div>
</template>
