<script setup lang="ts">
// Nouveau brouillon, en deux étapes : le prefab (recherche paginée côté API, le catalogue peut en
// compter des centaines), puis la version à copier (version suivante, images reprises). Le prefab
// d'exemple se crée dès la première étape.
import { Check, ChevronLeft, ChevronRight, Search, SearchX } from '@lucide/vue'
import { refDebounced } from '@vueuse/core'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { usePrefabSearch } from '@/features/prefabs/prefabs.queries'
import { useApiError } from '@/shared/api/useApiError'
import type { Schemas } from '@/shared/api/types'
import { cn } from '@/shared/lib/utils'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Skeleton } from '@/shared/ui/skeleton'

import { useStudioApi } from '../studio.api'
import { useInvalidateStudio } from '../studio.queries'

const PER_PAGE = 8

type Group = Schemas['PrefabGroupDto']

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const { t, locale } = useI18n()
const router = useRouter()
const api = useStudioApi()
const invalidate = useInvalidateStudio()
const { handleError, message } = useApiError()

const text = ref('')
const q = refDebounced(text, 250)
const page = ref(1)
watch(q, () => (page.value = 1))
const params = computed(() => ({ q: q.value.trim(), page: page.value, per_page: PER_PAGE }))
const { data: results, error, isFetching } = usePrefabSearch(params, () => props.open)
const pages = computed(() => Math.max(1, Math.ceil((results.value?.total ?? 0) / PER_PAGE)))

/** Étape 2 : le prefab choisi et la version à copier. */
const picked = ref<Group | null>(null)
const version = ref<string>()
const pending = ref(false)

const name = (group: Pick<Group, 'id' | 'names'>) => group.names[locale.value] ?? group.names.en ?? group.id

function pick(group: Group) {
  picked.value = group
  version.value = group.versions[0]
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    text.value = ''
    page.value = 1
    picked.value = null
  },
)

async function create(from?: { id: string; version: string }) {
  pending.value = true
  try {
    const draft = await api.create(from ? { from } : {})
    await invalidate.drafts()
    emit('update:open', false)
    await router.push(`/prefabs/studio/${draft.id}`)
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = false
  }
}

function confirm() {
  if (picked.value && version.value) void create({ id: picked.value.id, version: version.value })
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="sm:max-w-lg">
      <DialogHeader>
        <DialogTitle>{{ t('studio.new.title') }}</DialogTitle>
        <DialogDescription>{{ picked ? t('studio.new.versionHelp', { name: name(picked) }) : t('studio.new.help') }}</DialogDescription>
      </DialogHeader>

      <template v-if="!picked">
        <div class="relative">
          <Search class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input v-model="text" type="search" class="pl-8" :placeholder="t('studio.new.search')" :aria-label="t('studio.new.search')" />
        </div>
        <p v-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3 text-sm">{{ message(error) }}</p>
        <div v-else-if="!results" class="grid gap-1.5"><Skeleton v-for="index in 4" :key="index" class="h-12" /></div>
        <p v-else-if="results.items.length === 0" class="flex flex-col items-center gap-2 py-6 text-sm text-muted-foreground">
          <SearchX class="size-5" />{{ t('studio.new.noResult') }}
        </p>
        <ul v-else :class="cn('grid gap-1 transition-opacity', isFetching && 'opacity-60')" :aria-label="t('studio.new.prefabs')">
          <li v-for="group in results.items" :key="group.id">
            <button
              type="button"
              class="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
              @click="pick(group)"
            >
              <span class="min-w-0 flex-1">
                <span class="block truncate text-sm font-medium">{{ name(group) }}</span>
                <span class="block truncate font-mono text-xs text-muted-foreground">{{ group.id }}</span>
              </span>
              <span class="shrink-0 text-xs text-muted-foreground">{{ t('studio.new.versions', group.versions.length) }}</span>
              <ChevronRight class="size-4 shrink-0 text-muted-foreground" />
            </button>
          </li>
        </ul>
        <div v-if="results && pages > 1" class="flex items-center justify-between text-sm text-muted-foreground">
          <Button variant="ghost" size="icon" :disabled="page <= 1" :aria-label="t('dataTable.previous')" @click="page--"><ChevronLeft /></Button>
          <span>{{ t('dataTable.page', { page, count: pages }) }}</span>
          <Button variant="ghost" size="icon" :disabled="page >= pages" :aria-label="t('dataTable.next')" @click="page++"><ChevronRight /></Button>
        </div>
        <DialogFooter class="sm:justify-between">
          <Button type="button" variant="outline" :disabled="pending" @click="create()">{{ t('studio.new.blank') }}</Button>
          <Button type="button" variant="ghost" @click="emit('update:open', false)">{{ t('common.cancel') }}</Button>
        </DialogFooter>
      </template>

      <form v-else class="grid gap-3" @submit.prevent="confirm">
        <div role="radiogroup" :aria-label="t('studio.new.version')" class="grid max-h-72 gap-1 overflow-y-auto">
          <button
            v-for="(entry, index) in picked.versions"
            :key="entry"
            type="button"
            role="radio"
            :aria-checked="version === entry"
            :class="
              cn(
                'flex items-center gap-3 rounded-lg border px-3 py-2 text-left text-sm',
                version === entry ? 'border-primary bg-primary/10' : 'border-transparent hover:bg-accent',
              )
            "
            @click="version = entry"
          >
            <span class="flex-1 font-mono">{{ entry }}</span>
            <span v-if="index === 0" class="text-xs text-muted-foreground">{{ t('studio.new.latest') }}</span>
            <Check v-if="version === entry" class="size-4 text-primary" />
          </button>
        </div>
        <DialogFooter class="sm:justify-between">
          <Button type="button" variant="ghost" @click="picked = null"><ChevronLeft />{{ t('common.back') }}</Button>
          <Button type="submit" :disabled="pending || !version">{{ t('studio.new.create') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
