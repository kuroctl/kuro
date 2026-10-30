<script setup lang="ts">
// Édition d'un brouillon du Studio : fichiers (manifest, traductions, gabarits) avec
// autocomplétion et validation pendant la saisie, aperçu du formulaire de réglages, images ;
// enregistrement (même invalide), publication dans le catalogue, suppression.
import { autocompletion } from '@codemirror/autocomplete'
import { lintGutter } from '@codemirror/lint'
import type { Extension } from '@codemirror/state'
import { useDebounceFn } from '@vueuse/core'
import { AlertTriangle, CheckCircle2, Download, FileCode2, FileJson, FlaskConical, Languages, Plus, Rocket, Save, Trash2, X } from '@lucide/vue'
import { computed, reactive, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { onBeforeRouteLeave, useRoute, useRouter } from 'vue-router'
import { toast } from 'vue-sonner'
import { parse as parseYaml } from 'yaml'

import { usePermissions } from '@/features/account/account.queries'
import CodeEditor from '@/features/files/components/CodeEditor.vue'
import NameDialog from '@/features/files/components/NameDialog.vue'
import type { Manifest } from '@/features/prefabs/manifest'
import type { PrefabAssetKind, PrefabMessages } from '@/features/prefabs/prefabs.api'
import { useInvalidatePrefabs } from '@/features/prefabs/prefabs.queries'
import DraftAssetSlot from '@/features/studio/components/DraftAssetSlot.vue'
import DraftPreview from '@/features/studio/components/DraftPreview.vue'
import TrialDialog from '@/features/studio/components/TrialDialog.vue'
import { diagnosticsFor, locate, schemaCompletion } from '@/features/studio/manifestEditor'
import { type DraftCheck, type DraftFiles, useStudioApi } from '@/features/studio/studio.api'
import { useDraft, useInvalidateStudio, useStudioSchema } from '@/features/studio/studio.queries'
import { problemCode, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card } from '@/shared/ui/card'
import { Skeleton } from '@/shared/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/ui/tabs'

const MANIFEST = 'prefab.yaml'
const KINDS: PrefabAssetKind[] = ['icon', 'cover', 'thumbnail']

const { t, locale: language } = useI18n()
const route = useRoute('/prefabs/studio/[id]')
const router = useRouter()
const id = computed(() => route.params.id)
const { can } = usePermissions()
const api = useStudioApi()
const invalidate = useInvalidateStudio()
const invalidatePrefabs = useInvalidatePrefabs()
const { handleError } = useApiError()
const { data: draft, isPending, error } = useDraft(id)
const { data: schema } = useStudioSchema()

// Fichiers en cours d'édition ; `saved` est l'état enregistré, pour savoir s'il reste du travail.
const files = reactive<DraftFiles>({})
const saved = ref('')
const assets = ref<string[]>([])
const check = ref<DraftCheck>()
const selected = ref(MANIFEST)
const tab = ref('editor')
const editor = useTemplateRef<InstanceType<typeof CodeEditor>>('editor')

watch(
  draft,
  (loaded) => {
    if (!loaded || saved.value) return
    Object.assign(files, loaded.files)
    saved.value = JSON.stringify(loaded.files)
    assets.value = [...loaded.assets]
    check.value = loaded.check
  },
  { immediate: true },
)

const dirty = computed(() => Boolean(saved.value) && JSON.stringify(files) !== saved.value)
const paths = computed(() =>
  Object.keys(files).sort((a, b) => Number(b === MANIFEST) - Number(a === MANIFEST) || a.localeCompare(b)),
)
const format = (path: string) => (path.endsWith('.yaml') || path.endsWith('.yml') ? 'yaml' : path.endsWith('.json') ? 'json' : undefined)
const icon = (path: string) => (path === MANIFEST ? FileCode2 : path.startsWith('locales/') ? Languages : FileJson)
const problems = computed(() => (check.value?.reason ? 1 : 0) + (check.value?.issues.length ?? 0))

// Validation pendant la saisie : rien n'est enregistré.
const checking = ref(false)
const validate = useDebounceFn(async () => {
  checking.value = true
  try {
    check.value = await api.validate({ ...files })
  } catch (caught) {
    const data = (caught as { response?: { data?: { code?: string; params?: { reason?: string } } } }).response?.data
    if (data?.code === 'invalid_prefab') check.value = { valid: false, reason: data.params?.reason ?? '', issues: [] }
    else handleError(caught)
  } finally {
    checking.value = false
  }
}, 500)
watch(
  () => JSON.stringify(files),
  (_, previous) => {
    if (previous && saved.value) void validate()
  },
)

function edit(text: string) {
  files[selected.value] = text
}

const extensions = computed<Extension[]>(() => [
  lintGutter(),
  ...(selected.value === MANIFEST && schema.value ? [autocompletion({ override: [schemaCompletion(schema.value)] })] : []),
])

watch(
  [check, selected, () => editor.value],
  () => editor.value?.setDiagnostics(diagnosticsFor(selected.value, files[selected.value] ?? '', check.value)),
  { flush: 'post' },
)

function reveal(path?: string) {
  tab.value = 'editor'
  selected.value = MANIFEST
  const range = path ? locate(files[MANIFEST] ?? '', path) : undefined
  setTimeout(() => editor.value?.reveal(range?.from ?? 0))
}

// Aperçu : dernier manifest valide, traductions lisibles.
const lastValid = shallowRef<Manifest>()
watch(
  check,
  (current) => {
    if (!current?.valid) return
    try {
      lastValid.value = parseYaml(files[MANIFEST] ?? '') as Manifest
    } catch {
      // Valide côté API mais illisible ici : l'aperçu garde l'état précédent.
    }
  },
  { immediate: true },
)
const messages = computed(() => {
  const all: Record<string, PrefabMessages> = {}
  for (const [path, text] of Object.entries(files)) {
    const lang = /^locales\/([^/]+)\.json$/.exec(path)?.[1]
    if (!lang) continue
    try {
      all[lang] = JSON.parse(text) as PrefabMessages
    } catch {
      // Traduction en cours d'écriture : ignorée dans l'aperçu.
    }
  }
  return all
})

// Fichiers : ajout d'une langue ou d'un gabarit, retrait.
const adding = ref<'locale' | 'template' | null>(null)
async function addFile(name: string): Promise<string | undefined> {
  if (adding.value === 'locale' && !/^[a-z]{2,3}(-[A-Z]{2})?$/.test(name)) return t('studio.files.invalidLocale')
  const path = adding.value === 'locale' ? `locales/${name}.json` : `templates/${name}`
  if (path in files) return t('studio.files.exists')
  files[path] = adding.value === 'locale' ? '{\n  "prefab": { "name": "" }\n}\n' : ''
  selected.value = path
  tab.value = 'editor'
  return undefined
}
function removeFile(path: string) {
  delete files[path]
  if (selected.value === path) selected.value = MANIFEST
}

// Enregistrement, publication, suppression.
const saving = ref(false)
async function save() {
  if (!dirty.value || saving.value) return
  saving.value = true
  try {
    const stored = await api.save(id.value, { ...files })
    saved.value = JSON.stringify(stored.files)
    check.value = stored.check
    invalidate.store(stored)
    await invalidate.drafts()
    toast.success(t('studio.saved'))
  } catch (caught) {
    handleError(caught)
  } finally {
    saving.value = false
  }
}

const publishing = ref(false)
const confirmPublish = ref(false)
async function publish() {
  publishing.value = true
  try {
    const prefab = await api.publish(id.value)
    saved.value = ''
    confirmPublish.value = false
    await Promise.all([invalidate.drafts(), invalidatePrefabs()])
    toast.success(t('studio.published', { prefab: `${prefab.id} ${prefab.version}` }))
    await router.push({ path: `/prefabs/${prefab.id}`, query: { version: prefab.version } })
  } catch (caught) {
    confirmPublish.value = false
    if (problemCode(caught) === 'invalid_prefab') {
      const data = (caught as { response?: { data?: { params?: { reason?: string; issues?: DraftCheck['issues'] } } } }).response?.data
      check.value = { valid: false, reason: data?.params?.reason === 'invalid' ? null : (data?.params?.reason ?? null), issues: data?.params?.issues ?? [] }
    }
    handleError(caught)
  } finally {
    publishing.value = false
  }
}

const confirmDelete = ref(false)
async function remove() {
  try {
    await api.remove(id.value)
    saved.value = ''
    confirmDelete.value = false
    await invalidate.drafts()
    toast.success(t('studio.deleted'))
    await router.push('/prefabs/studio')
  } catch (caught) {
    handleError(caught)
  }
}

// Essai sur un nœud de test : le brouillon tel qu'il est enregistré.
const trying = ref(false)
const trialEnds = computed(() =>
  draft.value?.trial ? new Date(draft.value.trial.expires_at).toLocaleTimeString(language.value, { timeStyle: 'short' }) : '',
)

// Images du jeu Steam déclaré par le manifest **enregistré** (`steam.app`), comme l'API.
const steamApp = computed(() => {
  try {
    const stored = JSON.parse(saved.value || '{}') as DraftFiles
    const app = (parseYaml(stored[MANIFEST] ?? '') as Partial<Manifest> | null)?.steam?.app
    return typeof app === 'number' && app > 0 ? app : undefined
  } catch {
    return undefined
  }
})
const importingSteam = ref(false)
async function importSteam() {
  importingSteam.value = true
  try {
    const result = await api.importSteamAssets(id.value)
    assets.value = [...new Set([...assets.value, ...result.imported])]
    await invalidate.assets(id.value)
    const label = (kind: string) => t(`prefabs.detail.assetKinds.${kind}`)
    const missing = result.missing.map(label).join(', ')
    const grid = Object.entries(result.sources)
      .filter(([, source]) => source === 'steamgriddb')
      .map(([kind]) => label(kind))
      .join(', ')
    const description = grid ? t('studio.assets.steam.fromGrid', { kinds: grid }) : undefined
    if (missing) toast.warning(t('studio.assets.steam.partial', { game: result.name, missing }), { description })
    else toast.success(t('studio.assets.steam.done', { game: result.name }), { description })
  } catch (caught) {
    handleError(caught)
  } finally {
    importingSteam.value = false
  }
}

// Quitter avec des modifications non enregistrées : confirmation.
const leaving = ref<((leave: boolean) => void) | null>(null)
onBeforeRouteLeave(() => (dirty.value ? new Promise<boolean>((resolve) => (leaving.value = resolve)) : true))
function answerLeave(leave: boolean) {
  leaving.value?.(leave)
  leaving.value = null
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <PageHeader :title="draft?.prefab_id ?? t('studio.unnamed')">
      <template #meta>
        <span v-if="draft?.prefab_version" class="font-mono">{{ draft.prefab_version }}</span>
        <span v-if="draft?.base_id">{{ t('studio.basedOn', { prefab: `${draft.base_id}@${draft.base_version}` }) }}</span>
      </template>
      <template v-if="draft && can('prefabs.manage')">
        <Badge v-if="dirty" variant="secondary">{{ t('files.editor.unsaved') }}</Badge>
        <Button variant="outline" :disabled="!dirty || saving" @click="save"><Save />{{ t('common.save') }}</Button>
        <Button
          v-if="can('servers.create')"
          variant="outline"
          :disabled="dirty || !check?.valid"
          :title="dirty ? t('studio.publishSaveFirst') : t('studio.trial.help')"
          @click="trying = true"
        >
          <FlaskConical />{{ t('studio.trial.action') }}
        </Button>
        <Button :disabled="dirty || !check?.valid || publishing" :title="dirty ? t('studio.publishSaveFirst') : undefined" @click="confirmPublish = true">
          <Rocket />{{ t('studio.publish') }}
        </Button>
        <Button variant="ghost" size="icon" :aria-label="t('studio.delete')" @click="confirmDelete = true"><Trash2 /></Button>
      </template>
    </PageHeader>

    <p v-if="error" class="p-6 text-sm text-muted-foreground">{{ t('studio.notFound') }}</p>
    <div v-else-if="isPending || !draft" class="grid gap-4 p-6"><Skeleton class="h-96" /></div>
    <div v-else class="grid min-h-0 flex-1 gap-4 p-6 lg:grid-cols-[15rem_minmax(0,1fr)]">
      <Card class="gap-0 self-start py-0">
        <nav :aria-label="t('studio.files.title')" class="grid gap-0.5 p-2">
          <p class="px-2 pt-2 pb-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">{{ t('studio.files.title') }}</p>
          <div v-for="path in paths" :key="path" class="group flex items-center">
            <button
              type="button"
              class="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-muted/70"
              :class="{ 'bg-muted font-medium': selected === path && tab === 'editor' }"
              :aria-current="selected === path ? 'true' : undefined"
              @click="selected = path; tab = 'editor'"
            >
              <component :is="icon(path)" class="size-4 shrink-0 text-muted-foreground" />
              <span class="truncate font-mono text-xs">{{ path }}</span>
            </button>
            <Button
              v-if="path !== MANIFEST"
              variant="ghost"
              size="icon"
              class="size-7 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
              :aria-label="t('studio.files.remove', { path })"
              @click="removeFile(path)"
            >
              <X />
            </Button>
          </div>
          <div class="mt-2 grid gap-1 border-t pt-2">
            <Button variant="ghost" size="sm" class="justify-start" @click="adding = 'locale'"><Plus />{{ t('studio.files.addLocale') }}</Button>
            <Button variant="ghost" size="sm" class="justify-start" @click="adding = 'template'"><Plus />{{ t('studio.files.addTemplate') }}</Button>
          </div>
        </nav>
      </Card>

      <Tabs v-model="tab" class="min-h-0 min-w-0">
        <p v-if="draft.trial" class="flex flex-wrap items-center gap-2 rounded-lg border border-info/30 bg-info/8 px-3 py-2 text-sm" data-slot="draft-trial">
          <FlaskConical class="size-4 text-info" />
          <i18n-t keypath="studio.trial.running" tag="span" scope="global">
            <template #server>
              <RouterLink :to="`/servers/${draft.trial.server_id}`" class="font-semibold underline-offset-4 hover:underline">{{ draft.trial.name }}</RouterLink>
            </template>
            <template #time>{{ trialEnds }}</template>
          </i18n-t>
        </p>
        <TabsList>
          <TabsTrigger value="editor">{{ t('studio.tabs.editor') }}</TabsTrigger>
          <TabsTrigger value="preview">{{ t('studio.tabs.preview') }}</TabsTrigger>
          <TabsTrigger value="assets">{{ t('studio.tabs.assets') }}</TabsTrigger>
        </TabsList>

        <TabsContent value="editor" class="grid content-start gap-4">
          <Card class="flex h-[60vh] min-h-[24rem] flex-col gap-0 overflow-hidden py-0">
            <p class="border-b px-4 py-2 font-mono text-sm">{{ selected }}</p>
            <CodeEditor
              ref="editor"
              :key="`${selected}-${Boolean(schema)}`"
              :initial="files[selected] ?? ''"
              :format="format(selected)"
              :label="t('studio.editorLabel', { path: selected })"
              :extensions="extensions"
              @input="edit"
              @save="save"
            />
          </Card>
          <section :aria-label="t('studio.check.title')" class="grid gap-2 rounded-xl border bg-card p-4 shadow-card" data-slot="draft-check">
            <p class="flex items-center gap-2 text-sm font-semibold">
              <CheckCircle2 v-if="check?.valid" class="size-4 text-success" />
              <AlertTriangle v-else class="size-4 text-warning" />
              {{ check?.valid ? t('studio.check.valid') : t('studio.check.problems', problems) }}
              <span v-if="checking" class="text-xs font-normal text-muted-foreground">{{ t('studio.check.checking') }}</span>
            </p>
            <p v-if="check?.reason" class="font-mono text-xs whitespace-pre-wrap text-destructive">{{ check.reason }}</p>
            <ul v-if="check?.issues.length" class="grid gap-1">
              <li v-for="issue in check.issues" :key="`${issue.path}:${issue.message}`">
                <button type="button" class="text-left font-mono text-xs hover:underline" @click="reveal(issue.path)">
                  <span class="text-foreground">{{ issue.path }}</span><span class="text-muted-foreground">{{ ` : ${issue.message}` }}</span>
                </button>
              </li>
            </ul>
          </section>
        </TabsContent>

        <TabsContent value="preview">
          <div class="max-w-2xl">
            <DraftPreview v-if="lastValid" :manifest="lastValid" :messages="messages" />
            <p v-else class="text-sm text-muted-foreground">{{ t('studio.previewUnavailable') }}</p>
          </div>
        </TabsContent>

        <!-- `content-start` : `TabsContent` est `flex-1`, ses lignes ne s'étirent pas sur la hauteur restante. -->
        <TabsContent value="assets" class="grid content-start gap-4">
          <p class="max-w-3xl text-sm text-muted-foreground">{{ t('studio.assets.help') }}</p>
          <div v-if="steamApp && can('prefabs.manage')" class="flex flex-wrap items-center gap-3">
            <Button variant="outline" :disabled="importingSteam" @click="importSteam">
              <Download />{{ t('studio.assets.steam.action', { app: steamApp }) }}
            </Button>
            <span class="text-sm text-muted-foreground">{{ t('studio.assets.steam.help') }}</span>
          </div>
          <div class="grid items-start gap-4 md:grid-cols-3">
            <DraftAssetSlot
              v-for="kind in KINDS"
              :key="kind"
              :draft="id"
              :kind="kind"
              :present="assets.includes(kind)"
              @changed="(present) => (assets = present ? [...new Set([...assets, kind])] : assets.filter((entry) => entry !== kind))"
            />
          </div>
        </TabsContent>
      </Tabs>
    </div>

    <NameDialog
      :open="adding !== null"
      :title="adding === 'locale' ? t('studio.files.addLocale') : t('studio.files.addTemplate')"
      :description="adding === 'locale' ? t('studio.files.localeHelp') : t('studio.files.templateHelp')"
      :action="t('studio.files.add')"
      :submit="addFile"
      @update:open="(open) => !open && (adding = null)"
    />

    <TrialDialog
      v-if="draft"
      v-model:open="trying"
      :draft="id"
      :prefab-id="draft.prefab_id ?? ''"
      :replaces="Boolean(draft.trial)"
    />

    <AlertDialog :open="confirmPublish" @update:open="confirmPublish = $event">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('studio.publishTitle', { prefab: `${draft?.prefab_id} ${draft?.prefab_version}` }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('studio.publishHelp') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <Button :disabled="publishing" @click="publish"><Rocket />{{ t('studio.publish') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

    <AlertDialog :open="confirmDelete" @update:open="confirmDelete = $event">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('studio.deleteTitle') }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('studio.deleteHelp') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <Button variant="destructive" @click="remove"><Trash2 />{{ t('studio.delete') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

    <AlertDialog :open="leaving !== null">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('studio.leaveTitle') }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('studio.leaveHelp') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button variant="ghost" @click="answerLeave(false)">{{ t('studio.stay') }}</Button>
          <Button variant="destructive" @click="answerLeave(true)">{{ t('studio.leave') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
