<script setup lang="ts">
// Recherche chez une source de l'emplacement (Modrinth, CurseForge ; filtrée par l'API selon le
// loader et la version du jeu), triée et filtrée par catégories de la source, puis choix de la
// version et aperçu des dépendances installées avec elle. Source, tri et catégories restent dans
// l'URL. Une source sans clé d'API renvoie vers son réglage ; un projet dont l'auteur interdit la
// distribution par des tiers renvoie vers sa page. Les valeurs à saisir sont pré-remplies d'après
// la description du projet (identifiants de mods du Workshop), à faire vérifier.
import { Download, ExternalLink, KeyRound, Package, Search } from '@lucide/vue'
import { refDebounced } from '@vueuse/core'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { DataTableFacetFilter } from '@/shared/ui/data-table'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Skeleton } from '@/shared/ui/skeleton'

import { type ContentSlot, type ContentSort, type ProjectHit, type SearchFilters, useContentApi } from '../content.api'
import {
  useContentCategories,
  useContentProject,
  useContentSearch,
  useContentVersions,
  useInstalledContent,
  useInvalidateContent,
} from '../content.queries'
import { valuesPayload, valuesTexts } from '../values'
import ContentValuesForm from './ContentValuesForm.vue'

const SORTS: ContentSort[] = ['relevance', 'downloads', 'updated', 'newest']

const props = defineProps<{
  server: string
  contentSlot: string
  sources: ContentSlot['sources']
  reference: boolean
  canInstall: boolean
  /** Serveur en marche : l'ajout est appliqué au prochain redémarrage. */
  live?: boolean
  /** Valeurs à saisir pour chaque contenu, libellées par le prefab. */
  fields: ContentSlot['fields']
  fieldLabel: (field: string) => string
  fieldHelp: (field: string) => string | undefined
}>()
const emit = defineEmits<{ installed: [] }>()

const { t, locale } = useI18n()
const { can } = usePermissions()
const api = useContentApi()
const { handleError, message } = useApiError()
const invalidate = useInvalidateContent()

const route = useRoute()
const router = useRouter()
const queryValue = (key: string) => (typeof route.query[key] === 'string' ? (route.query[key] as string) : '')

/** Source parcourue : celle de l'URL si l'emplacement la propose, sinon la première utilisable. */
const source = computed(() => {
  const wanted = queryValue('source')
  if (props.sources.some((s) => s.provider === wanted)) return wanted
  return props.sources.find((s) => !s.blocked)?.provider ?? props.sources[0]?.provider ?? 'modrinth'
})
const filters = computed<SearchFilters>(() => ({
  sort: (SORTS as string[]).includes(queryValue('sort')) ? (queryValue('sort') as ContentSort) : 'relevance',
  categories: queryValue('categories').split(',').filter(Boolean),
}))

/** Garde la vue dans l'URL ; les valeurs par défaut n'y figurent pas. */
function setQuery(changes: Record<string, string | undefined>) {
  const query = { ...route.query, ...changes }
  for (const [key, value] of Object.entries(query)) if (!value) delete query[key]
  void router.replace({ query })
}
function pickSource(provider: string) {
  picked.value = null
  // Les catégories sont propres à chaque source.
  setQuery({ source: provider, categories: undefined })
}
const setSort = (sort: ContentSort) => setQuery({ sort: sort === 'relevance' ? undefined : sort })
const setCategories = (categories: string[]) => setQuery({ categories: categories.join(',') || undefined })

const blocked = computed(() => props.sources.find((s) => s.provider === source.value)?.blocked ?? null)
const text = ref('')
const debounced = refDebounced(text, 300)
const { data: page, error, isFetching } = useContentSearch(
  () => props.server,
  () => props.contentSlot,
  source,
  debounced,
  filters,
  () => !blocked.value,
)
const { data: categories } = useContentCategories(
  () => props.server,
  () => props.contentSlot,
  source,
  () => !blocked.value,
)
const categoryOptions = computed(() => (categories.value ?? []).map((c) => ({ value: c.id, label: c.name })))
const { data: installed } = useInstalledContent(
  () => props.server,
  () => props.contentSlot,
)
const installedProjects = computed(
  () =>
    new Set(
      (installed.value?.items ?? []).filter((item) => item.state !== 'failed' && item.source === source.value).map((item) => item.project_id),
    ),
)

const picked = ref<ProjectHit | null>(null)
const { data: versions, isPending: loadingVersions } = useContentVersions(
  () => props.server,
  () => props.contentSlot,
  source,
  () => picked.value?.project_id,
)
/** Téléchargé par le nœud : l'auteur doit l'autoriser. En mode `reference`, le jeu s'en charge. */
const installable = (hit: ProjectHit) => props.reference || hit.download_allowed
const chosen = ref<string>()
watch(versions, (list) => (chosen.value = list?.[0]?.id))
const version = computed(() => versions.value?.find((v) => v.id === chosen.value))
const pending = ref(false)
const values = ref<Record<string, string>>({})
const valueErrors = ref<FieldErrors>({})
const { data: project } = useContentProject(
  () => props.server,
  () => props.contentSlot,
  source,
  () => (props.fields.some((field) => field.prefill) ? picked.value?.project_id : undefined),
)
/** Valeurs proposées par la description du projet choisi, s'il y en a. */
const prefill = computed(() =>
  project.value && project.value.project_id === picked.value?.project_id && Object.keys(project.value.prefill).length
    ? project.value.prefill
    : undefined,
)
/** La description a été lue sans rien y trouver : la saisie revient à l'administrateur. */
const prefillMissed = computed(() => !!project.value && project.value.project_id === picked.value?.project_id && !prefill.value)
watch(picked, () => {
  values.value = valuesTexts(props.fields, prefill.value ?? {})
  valueErrors.value = {}
})
// La fiche arrive après l'ouverture : elle ne remplace que des champs encore vides.
watch(prefill, (proposed) => {
  if (!proposed) return
  const texts = valuesTexts(props.fields, proposed)
  for (const field of props.fields) if (!values.value[field.key]?.trim()) values.value[field.key] = texts[field.key] ?? ''
})

const count = (value: number) => new Intl.NumberFormat(locale.value, { notation: 'compact' }).format(value)

async function install() {
  // Un double clic part avant que le bouton soit désactivé : une seule installation à la fois.
  if (!version.value || !picked.value || pending.value) return
  pending.value = true
  try {
    const items = await api.install(props.server, props.contentSlot, source.value, version.value.id, valuesPayload(props.fields, values.value))
    if (props.live) {
      toast.success(t('content.addedLive', { name: picked.value.title }), { description: t('content.nextRestart') })
    } else {
      toast.success(
        props.reference ? t('content.referenced', { name: picked.value.title }) : t('content.installing', { name: picked.value.title, count: items.length - 1 }),
      )
    }
    picked.value = null
    emit('installed')
    await invalidate(props.server)
  } catch (failure) {
    valueErrors.value = handleError(
      failure,
      props.fields.map((field) => `values.${field.key}`),
    )
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <div class="grid gap-3">
    <div class="flex flex-wrap items-center gap-2">
      <div v-if="sources.length > 1" role="radiogroup" :aria-label="t('content.sources.label')" class="inline-flex rounded-lg border p-0.5">
        <button
          v-for="candidate in sources"
          :key="candidate.provider"
          type="button"
          role="radio"
          :aria-checked="candidate.provider === source"
          class="rounded-md px-3 py-1 text-sm transition-colors"
          :class="candidate.provider === source ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:text-foreground'"
          @click="pickSource(candidate.provider)"
        >
          {{ t(`content.sources.${candidate.provider}`) }}
        </button>
      </div>
      <div class="relative max-w-md min-w-48 flex-1">
        <Search class="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input v-model="text" type="search" class="pl-8" :placeholder="t('content.search')" :aria-label="t('content.search')" :disabled="!!blocked" />
      </div>
    </div>
    <div v-if="!blocked" class="flex flex-wrap items-center gap-2">
      <Select :model-value="filters.sort" @update:model-value="(value) => setSort(value as ContentSort)">
        <SelectTrigger size="sm" class="h-8 w-auto" :aria-label="t('content.filters.sort')">
          <SelectValue>{{ t('content.filters.sortBy', { sort: t(`content.filters.sorts.${filters.sort}`) }) }}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem v-for="sort in SORTS" :key="sort" :value="sort">{{ t(`content.filters.sorts.${sort}`) }}</SelectItem>
        </SelectContent>
      </Select>
      <DataTableFacetFilter
        v-if="categoryOptions.length"
        :model-value="filters.categories"
        :title="t('content.filters.categories')"
        :options="categoryOptions"
        @update:model-value="setCategories"
      />
    </div>
    <Alert v-if="blocked === 'content_source_not_configured'" variant="warning" class="flex-wrap items-center">
      <KeyRound />
      <span class="flex-1">{{ t('content.sources.noKey', { source: t(`content.sources.${source}`) }) }}</span>
      <Button v-if="can('system.content_sources.manage')" size="sm" variant="outline" as-child>
        <RouterLink to="/settings/content-sources">{{ t('content.sources.configure') }}</RouterLink>
      </Button>
    </Alert>
    <p v-else-if="blocked" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t(`errors.${blocked}`) }}</p>
    <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
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
            <span v-if="hit.author" class="text-xs font-normal text-muted-foreground">{{ t('content.by', { author: hit.author }) }}</span>
          </p>
          <p class="line-clamp-2 text-xs text-muted-foreground">{{ hit.description }}</p>
          <p class="text-xs text-muted-foreground">{{ t('content.downloads', { count: count(hit.downloads) }) }}</p>
        </div>
        <span v-if="installedProjects.has(hit.project_id)" class="text-xs text-muted-foreground">{{ t('content.alreadyInstalled') }}</span>
        <div v-else-if="!installable(hit)" class="grid justify-items-end gap-1 text-right">
          <span class="text-xs text-muted-foreground">{{ t('content.sources.forbidden') }}</span>
          <a v-if="hit.page_url" :href="hit.page_url" target="_blank" rel="noopener noreferrer" class="inline-flex items-center gap-1 text-xs underline">
            {{ t('content.sources.page') }}<ExternalLink class="size-3" />
          </a>
        </div>
        <Button v-else-if="canInstall" size="sm" variant="outline" @click="picked = hit"><Download />{{ t('content.add') }}</Button>
      </li>
    </ul>

    <Dialog :open="picked !== null" @update:open="(open) => !open && (picked = null)">
      <DialogContent class="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{{ t('content.installTitle', { name: picked?.title ?? '' }) }}</DialogTitle>
          <DialogDescription>{{ reference ? t('content.referenceHelp') : t('content.installHelp') }}</DialogDescription>
        </DialogHeader>
        <Skeleton v-if="loadingVersions" class="h-24" />
        <p v-else-if="!versions?.length" class="text-sm text-muted-foreground">{{ t('content.noVersion') }}</p>
        <div v-else class="grid gap-3">
          <label class="grid gap-1.5 text-sm">
            <span class="font-medium">{{ t('content.version') }}</span>
            <select v-model="chosen" class="h-9 rounded-md border bg-transparent px-2 text-sm" :aria-label="t('content.version')">
              <option v-for="v in versions" :key="v.id" :value="v.id" class="bg-background">{{ [v.version_number, ...v.loaders].join(' · ') }}</option>
            </select>
          </label>
          <p v-if="version?.required_projects.length" class="text-sm text-muted-foreground">
            {{ t('content.withDependencies', { count: version.required_projects.length }) }}
          </p>
          <p v-if="!reference" class="font-mono text-xs text-muted-foreground">{{ version?.file_name }}</p>
          <template v-if="fields.length">
            <p v-if="prefill" class="text-xs text-muted-foreground">{{ t('content.prefilled') }}</p>
            <p v-else-if="prefillMissed" class="text-xs text-muted-foreground">
              {{ t('content.prefillMissed') }}
              <a v-if="picked?.page_url" :href="picked.page_url" target="_blank" rel="noopener noreferrer" class="inline-flex items-center gap-1 underline">
                {{ t('content.sources.page') }}<ExternalLink class="size-3" />
              </a>
            </p>
            <ContentValuesForm v-model="values" :fields="fields" :label="fieldLabel" :help="fieldHelp" :errors="valueErrors" />
          </template>
        </div>
        <DialogFooter>
          <Button variant="ghost" @click="picked = null">{{ t('common.cancel') }}</Button>
          <Button :disabled="pending || !version" @click="install">{{ t('content.install') }}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </div>
</template>
