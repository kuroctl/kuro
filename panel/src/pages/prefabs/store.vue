<script setup lang="ts">
// Store de prefabs (ADR-0042) : les versions publiées par les dépôts de prefabs configurés
// (index signés, archives vérifiées par leur somme), leur compatibilité avec cette API et leur
// présence au catalogue ; installation en un clic (`prefabs.manage`).
import { Download, RefreshCw, Store } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { usePrefabsApi } from '@/features/prefabs/prefabs.api'
import { useInvalidatePrefabs, usePrefabStores } from '@/features/prefabs/prefabs.queries'
import type { Schemas } from '@/shared/api/types'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { formatBytes } from '@/shared/lib/format'
import { Alert, AlertDescription, AlertTitle } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'

type StorePrefab = Schemas['StorePrefabDto']

const { t, locale } = useI18n()
const router = useRouter()
const { can } = usePermissions()
const api = usePrefabsApi()
const invalidate = useInvalidatePrefabs()
const { handleError } = useApiError()
const { data: stores, isPending } = usePrefabStores(() => can('prefabs.read'))

const name = (prefab: StorePrefab) => prefab.names[locale.value] ?? prefab.names.en ?? prefab.id
const description = (prefab: StorePrefab) => prefab.descriptions[locale.value] ?? prefab.descriptions.en

// Relecture des index côté API (cache de dix minutes), puis de la liste.
const refreshing = ref(false)
async function refresh() {
  refreshing.value = true
  try {
    await api.stores(true)
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    refreshing.value = false
  }
}

const installing = ref<string | null>(null)
const key = (store: string, prefab: StorePrefab) => `${store}/${prefab.id}@${prefab.version}`

async function install(store: string, prefab: StorePrefab) {
  installing.value = key(store, prefab)
  try {
    const imported = await api.installFromStore(store, prefab.id, prefab.version)
    await invalidate()
    toast.success(t('prefabs.store.installed', { name: name(prefab), version: prefab.version }))
    await router.push({ path: `/prefabs/${imported.id}`, query: { version: imported.version } })
  } catch (error) {
    handleError(error)
  } finally {
    installing.value = null
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('prefabs.store.title')">
      <Button variant="outline" :disabled="refreshing" :title="t('prefabs.store.refreshHelp')" @click="refresh">
        <RefreshCw :class="{ 'animate-spin': refreshing }" />{{ t('prefabs.store.refresh') }}
      </Button>
    </PageHeader>
    <div class="grid gap-4 p-6">
      <p class="max-w-3xl text-sm text-muted-foreground">{{ t('prefabs.store.help') }}</p>

      <div v-if="isPending" class="grid gap-4"><Skeleton class="h-40" /><Skeleton class="h-40" /></div>
      <Empty v-else-if="!stores?.length" :icon="Store">{{ t('prefabs.store.none') }}</Empty>
      <Card v-for="store in stores" v-else :key="store.name" class="gap-0 py-0" :data-store="store.name">
        <CardHeader class="flex flex-wrap items-baseline gap-x-3 gap-y-1 px-5 py-4">
          <CardTitle>{{ store.name }}</CardTitle>
          <span class="truncate font-mono text-xs text-muted-foreground">{{ store.url }}</span>
        </CardHeader>
        <CardContent class="px-0 pb-0">
          <Alert v-if="store.error" variant="destructive" role="alert" class="mx-5 mb-5">
            <Store />
            <div class="grid gap-1">
              <AlertTitle>{{ t('prefabs.store.unreadable') }}</AlertTitle>
              <AlertDescription class="font-mono text-xs">{{ store.error }}</AlertDescription>
            </div>
          </Alert>
          <Empty v-else-if="store.prefabs.length === 0" :icon="Store" class="border-0 shadow-none">{{ t('prefabs.store.empty') }}</Empty>
          <ul v-else class="divide-y border-t">
            <li v-for="prefab in store.prefabs" :key="`${prefab.id}@${prefab.version}`" class="flex flex-wrap items-center gap-3 px-5 py-3" data-store-prefab>
              <span class="grid min-w-0 flex-1 gap-0.5">
                <span class="flex min-w-0 flex-wrap items-center gap-2">
                  <span class="truncate font-semibold">{{ name(prefab) }}</span>
                  <span class="font-mono text-xs text-muted-foreground">{{ `${prefab.id} · ${prefab.version}` }}</span>
                  <Badge variant="outline">{{ t(`prefabs.catalog.categories.${prefab.category}`) }}</Badge>
                  <Badge v-if="prefab.installed" variant="info">{{ t('prefabs.store.alreadyInstalled') }}</Badge>
                  <Badge v-else-if="!prefab.compatible" variant="secondary">{{ t('prefabs.store.incompatible', { requires: prefab.requires_kuro ?? '?' }) }}</Badge>
                  <Badge v-if="prefab.draft_reason !== null && prefab.draft_reason !== undefined" variant="secondary" :title="prefab.draft_reason">{{ t('prefabs.store.draft') }}</Badge>
                </span>
                <span v-if="description(prefab)" class="truncate text-sm text-muted-foreground">{{ description(prefab) }}</span>
              </span>
              <span class="text-xs text-muted-foreground">{{ formatBytes(prefab.size, locale) }}</span>
              <Button
                v-if="can('prefabs.manage')"
                size="sm"
                variant="outline"
                :disabled="prefab.installed || !prefab.compatible || installing !== null"
                @click="install(store.name, prefab)"
              >
                <Download :class="{ 'animate-pulse': installing === key(store.name, prefab) }" />{{ t('prefabs.store.install') }}
              </Button>
            </li>
          </ul>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
