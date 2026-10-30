<script setup lang="ts">
// Catalogue des prefabs (ADR-0011) : une carte par prefab (dernière version, nombre de versions
// et de serveurs, origine), import d'une archive `.kprefab` avec le détail d'un refus.
import { Boxes, RefreshCw, Upload } from '@lucide/vue'
import { isAxiosError } from 'axios'
import { computed, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import PrefabCard from '@/features/prefabs/components/PrefabCard.vue'
import { usePrefabsApi } from '@/features/prefabs/prefabs.api'
import { useInvalidatePrefabs, usePrefabs } from '@/features/prefabs/prefabs.queries'
import type { Schemas } from '@/shared/api/types'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { formatBytes } from '@/shared/lib/format'
import { Alert, AlertDescription, AlertTitle } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'

/** Doit suivre `kuro_prefab::files::MAX_ARCHIVE_BYTES`. */
const MAX_ARCHIVE_BYTES = 10 * 1024 * 1024

type Summary = Schemas['PrefabSummaryDto']

const { t, locale } = useI18n()
const router = useRouter()
const { can } = usePermissions()
const { data: prefabs, isPending } = usePrefabs(() => can('prefabs.read'))
const api = usePrefabsApi()
const invalidate = useInvalidatePrefabs()
const { handleError } = useApiError()

/** Un prefab par carte : sa version la plus récente, et le total de ses versions. */
const cards = computed(() => {
  const byId = new Map<string, Summary[]>()
  for (const prefab of prefabs.value ?? []) byId.set(prefab.id, [...(byId.get(prefab.id) ?? []), prefab])
  return [...byId.values()].map((versions) => {
    const latest = versions.reduce((a, b) => (b.imported_at > a.imported_at ? b : a))
    return { latest, versions: versions.length, servers: versions.reduce((sum, v) => sum + v.servers, 0) }
  })
})

const name = (prefab: Summary) => prefab.names[locale.value] ?? prefab.names.en ?? prefab.id

// Rechargement du dossier des prefabs officiels, sans redémarrer l'API.
const reloading = ref(false)

async function reload() {
  reloading.value = true
  try {
    const report = await api.reload()
    await invalidate()
    const invalid = report.invalid.map((entry) => entry.folder)
    if (invalid.length) toast.warning(t('prefabs.catalog.reloadIssues', { list: invalid.join(', ') }))
    if (report.conflicts.length) toast.warning(t('prefabs.catalog.reloadConflicts', { list: report.conflicts.join(', ') }))
    const issues = [...invalid, ...report.conflicts]
    if (report.imported.length || report.assets_added.length) {
      toast.success(t('prefabs.catalog.reloaded', { versions: report.imported.length, images: report.assets_added.length }))
    } else if (!issues.length) {
      toast.success(t('prefabs.catalog.unchangedAll'))
    }
  } catch (error) {
    handleError(error)
  } finally {
    reloading.value = false
  }
}

// Import : fichier choisi, envoi avec progression, refus détaillé.
const input = useTemplateRef<HTMLInputElement>('input')
const progress = ref<number | null>(null)
const refusal = ref<{ reason: string; issues: { path: string; message: string }[] } | null>(null)

async function upload(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]
  if (input.value) input.value.value = ''
  if (!file) return
  refusal.value = null
  if (file.size > MAX_ARCHIVE_BYTES) {
    toast.error(t('prefabs.catalog.tooLarge', { max: formatBytes(MAX_ARCHIVE_BYTES, locale.value) }))
    return
  }
  progress.value = 0
  try {
    const prefab = await api.importArchive(file, (ratio) => (progress.value = ratio))
    await invalidate()
    toast.success(t('prefabs.catalog.imported', { name: name(prefab), version: prefab.version }))
    await router.push(`/prefabs/${prefab.id}?version=${prefab.version}`)
  } catch (error) {
    const data = isAxiosError(error) ? error.response?.data : undefined
    if (data?.code === 'invalid_prefab') {
      refusal.value = { reason: data.params?.reason ?? '', issues: data.params?.issues ?? [] }
    } else {
      handleError(error)
    }
  } finally {
    progress.value = null
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('prefabs.catalog.title')">
      <template v-if="can('prefabs.manage')">
        <Button variant="outline" :disabled="reloading" :title="t('prefabs.catalog.reloadHelp')" @click="reload">
          <RefreshCw :class="{ 'animate-spin': reloading }" />{{ t('prefabs.catalog.reload') }}
        </Button>
        <input ref="input" type="file" accept=".kprefab,.zip" class="sr-only" :aria-label="t('prefabs.catalog.import')" @change="upload">
        <Button :disabled="progress !== null" @click="input?.click()">
          <Upload />
          {{ progress === null ? t('prefabs.catalog.import') : `${t('prefabs.catalog.importing')} ${Math.round(progress * 100)} %` }}
        </Button>
      </template>
    </PageHeader>
    <div class="grid gap-4 p-6">
      <p class="max-w-3xl text-sm text-muted-foreground">{{ t('prefabs.catalog.help') }}</p>

      <Alert v-if="refusal" variant="destructive" role="alert">
        <Boxes />
        <div class="grid gap-1">
          <AlertTitle>{{ t('prefabs.catalog.invalid') }}</AlertTitle>
          <AlertDescription>
            <p v-if="refusal.reason !== 'invalid'" class="font-mono text-xs">{{ refusal.reason }}</p>
            <ul v-if="refusal.issues.length" class="mt-1 grid gap-0.5 font-mono text-xs">
              <li v-for="issue in refusal.issues" :key="`${issue.path}:${issue.message}`">
                <span class="text-foreground">{{ issue.path }}</span>{{ ` : ${issue.message}` }}
              </li>
            </ul>
          </AlertDescription>
        </div>
      </Alert>

      <p v-if="!can('prefabs.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('prefabs.catalog.forbidden') }}
      </p>
      <div v-else-if="isPending" class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3"><Skeleton v-for="index in 3" :key="index" class="h-56" /></div>
      <Empty v-else-if="cards.length === 0" :icon="Boxes">{{ t('prefabs.catalog.empty') }}</Empty>
      <ul v-else class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
        <li v-for="card in cards" :key="card.latest.id" :data-prefab="card.latest.id">
          <RouterLink :to="`/prefabs/${card.latest.id}`" class="block h-full rounded-xl transition-transform hover:-translate-y-0.5 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none">
            <PrefabCard :prefab="card.latest" :versions="card.versions" :servers="card.servers" />
          </RouterLink>
        </li>
      </ul>
    </div>
  </div>
</template>
