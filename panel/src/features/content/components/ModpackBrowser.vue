<script setup lang="ts">
// Modpacks Modrinth (ADR-0020) : un modpack impose la version du jeu et le loader ; l'installer
// les règle (redéploiement) avant de déposer ses fichiers. Un seul modpack par emplacement.
import { Download, ExternalLink, Package, Search } from '@lucide/vue'
import { useQuery } from '@tanstack/vue-query'
import { refDebounced } from '@vueuse/core'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Skeleton } from '@/shared/ui/skeleton'

import { type ProjectHit, useContentApi } from '../content.api'
import { useInvalidateContent } from '../content.queries'

const props = defineProps<{ server: string; contentSlot: string; canInstall: boolean; installed: boolean }>()
const emit = defineEmits<{ installed: [] }>()

const { t, locale } = useI18n()
const api = useContentApi()
const { handleError, message } = useApiError()
const invalidate = useInvalidateContent()

const text = ref('')
const debounced = refDebounced(text, 300)
const { data: page, error, isFetching } = useQuery(
  computed(() => ({
    queryKey: ['servers', props.server, 'content', props.contentSlot, 'modpacks', debounced.value],
    queryFn: () => api.modpackSearch(props.server, props.contentSlot, debounced.value),
    staleTime: 60_000,
  })),
)

const picked = ref<ProjectHit | null>(null)
const { data: versions, isPending: loadingVersions } = useQuery(
  computed(() => ({
    queryKey: ['servers', props.server, 'content', props.contentSlot, 'modpacks', 'versions', picked.value?.project_id],
    queryFn: () => api.modpackVersions(props.server, props.contentSlot, picked.value!.project_id),
    enabled: !!picked.value,
    staleTime: 60_000,
  })),
)
const chosen = ref<string>()
watch(versions, (list) => (chosen.value = list?.[0]?.id))
const version = computed(() => versions.value?.find((v) => v.id === chosen.value))
const pending = ref(false)

const count = (value: number) => new Intl.NumberFormat(locale.value, { notation: 'compact' }).format(value)

async function install() {
  if (!version.value || !picked.value) return
  pending.value = true
  try {
    await api.installModpack(props.server, props.contentSlot, version.value.id)
    toast.success(t('content.packs.installing', { name: picked.value.title }))
    picked.value = null
    emit('installed')
    await invalidate(props.server)
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <div class="grid gap-3">
    <p v-if="installed" role="status" class="rounded-md border p-3 text-sm text-muted-foreground">{{ t('content.packs.alreadyOne') }}</p>
    <div class="relative max-w-md">
      <Search class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input v-model="text" type="search" class="pl-8" :placeholder="t('content.packs.search')" :aria-label="t('content.packs.search')" />
    </div>
    <p v-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
    <div v-else-if="!page" class="grid gap-2"><Skeleton v-for="index in 4" :key="index" class="h-16" /></div>
    <p v-else-if="page.hits.length === 0" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t('content.noResult') }}</p>
    <ul v-else class="grid gap-2" :class="{ 'opacity-60': isFetching }">
      <li v-for="hit in page.hits" :key="hit.project_id" class="flex items-center gap-3.5 rounded-xl border bg-card p-3 shadow-card">
        <img v-if="hit.icon_url" :src="hit.icon_url" alt="" class="size-[52px] rounded-[10px] bg-muted object-cover" loading="lazy" referrerpolicy="no-referrer">
        <div v-else class="flex size-[52px] items-center justify-center rounded-[10px] bg-muted"><Package class="size-5 text-muted-foreground" /></div>
        <div class="grid min-w-0 flex-1 gap-0.5">
          <p class="flex flex-wrap items-center gap-x-1.5 text-[15px] font-bold">
            {{ hit.title }}
            <a
              v-if="hit.page_url"
              :href="hit.page_url"
              target="_blank"
              rel="noopener noreferrer"
              class="text-muted-foreground hover:text-foreground"
              :aria-label="t('content.pageOf', { name: hit.title })"
              :title="t('content.pageOf', { name: hit.title })"
            ><ExternalLink class="size-3.5" /></a>
            <span class="text-xs font-normal text-muted-foreground">{{ t('content.by', { author: hit.author }) }}</span>
          </p>
          <p class="line-clamp-2 text-xs text-muted-foreground">{{ hit.description }}</p>
          <p class="text-xs text-muted-foreground">{{ t('content.downloads', { count: count(hit.downloads) }) }}</p>
        </div>
        <Button v-if="canInstall && !installed" size="sm" variant="outline" @click="picked = hit"><Download />{{ t('content.add') }}</Button>
      </li>
    </ul>

    <Dialog :open="picked !== null" @update:open="(open) => !open && (picked = null)">
      <DialogContent class="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{{ t('content.installTitle', { name: picked?.title ?? '' }) }}</DialogTitle>
          <DialogDescription>{{ t('content.packs.help') }}</DialogDescription>
        </DialogHeader>
        <Skeleton v-if="loadingVersions" class="h-24" />
        <p v-else-if="!versions?.length" class="text-sm text-muted-foreground">{{ t('content.noVersion') }}</p>
        <div v-else class="grid gap-3">
          <label class="grid gap-1.5 text-sm">
            <span class="font-medium">{{ t('content.version') }}</span>
            <select v-model="chosen" class="h-9 rounded-md border bg-transparent px-2 text-sm" :aria-label="t('content.version')">
              <option v-for="v in versions" :key="v.id" :value="v.id" class="bg-background">
                {{ `${v.version_number} · ${v.game_versions.join(', ')} · ${v.loaders.join(', ')}` }}
              </option>
            </select>
          </label>
          <p v-if="version" class="rounded-lg border border-warning/35 bg-warning/8 px-4 py-3.5 text-sm">
            {{ t('content.packs.sets', { version: version.game_versions.join(', '), loader: version.loaders.join(', ') }) }}
          </p>
        </div>
        <DialogFooter>
          <Button variant="ghost" @click="picked = null">{{ t('common.cancel') }}</Button>
          <Button :disabled="pending || !version" @click="install">{{ t('content.install') }}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </div>
</template>
