<script setup lang="ts">
// Contenu installé d'un emplacement : version, dépendance de, mise à jour, activation,
// suppression (avec les dépendances devenues inutiles, au choix). Les fichiers d'un modpack sont
// regroupés sous lui et partent avec lui. Un fichier peut aussi être envoyé depuis l'ordinateur.
import { ArrowUpCircle, ExternalLink, Package, SlidersHorizontal, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { formatBytes } from '@/shared/lib/format'
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
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'
import { Switch } from '@/shared/ui/switch'

import UploadButton from '@/features/files/components/UploadButton.vue'

import { type ContentItem, type ContentSlot, useContentApi } from '../content.api'
import { useInstalledContent, useInvalidateContent } from '../content.queries'
import { valuesPayload, valuesSummary, valuesTexts } from '../values'
import ContentValuesForm from './ContentValuesForm.vue'

/** `acceptsUpload` : le prefab accepte des fichiers envoyés depuis l'ordinateur. `fields` :
 * valeurs saisies pour chaque contenu (ADR-0030), libellées par le prefab. */
const props = withDefaults(
  defineProps<{
    /** Serveur en marche : un changement est appliqué au prochain redémarrage. */
    live?: boolean
    server: string
    contentSlot: string
    suspended?: boolean
    acceptsUpload?: boolean
    fields?: ContentSlot['fields']
    fieldLabel?: (field: string) => string
    fieldHelp?: (field: string) => string | undefined
  }>(),
  { fields: () => [], fieldLabel: (field: string) => field, fieldHelp: () => undefined },
)
const emit = defineEmits<{ changed: [] }>()

const { t, locale } = useI18n()
const { can } = usePermissions()
const api = useContentApi()
const { handleError } = useApiError()
const invalidate = useInvalidateContent()
const { data, isPending } = useInstalledContent(
  () => props.server,
  () => props.contentSlot,
)

const all = computed(() => data.value?.items ?? [])
/** Contenu de premier niveau : les fichiers d'un modpack sont listés sous lui. */
const items = computed(() => all.value.filter((item) => !item.modpack_id))
const filesOf = (pack: ContentItem) => all.value.filter((item) => item.modpack_id === pack.id)
const byId = computed(() => new Map(all.value.map((item) => [item.id, item])))
const manage = computed(() => can('servers.content.manage') && !props.suspended)
const busy = ref<string | null>(null)
const removing = ref<ContentItem | null>(null)
/** Décochée par défaut : un mod peut utiliser une dépendance sans la déclarer chez la source. */
const withOrphans = ref(false)

const names = (ids: string[]) => ids.map((id) => byId.value.get(id)?.name ?? '').filter(Boolean).join(', ')

async function run(item: ContentItem, action: () => Promise<unknown>, done: string) {
  busy.value = item.id
  try {
    await action()
    toast.success(t(done, { name: item.name }), props.live ? { description: t('content.nextRestart') } : undefined)
    emit('changed')
    await invalidate(props.server)
  } catch (error) {
    handleError(error)
  } finally {
    busy.value = null
  }
}

function toggle(item: ContentItem, enabled: boolean) {
  void run(item, () => api.toggle(props.server, props.contentSlot, item.id, enabled), enabled ? 'content.enabled' : 'content.disabled')
}

function update(item: ContentItem) {
  void run(item, () => api.update(props.server, props.contentSlot, item.id), 'content.updating')
}

function askRemove(item: ContentItem) {
  withOrphans.value = false
  removing.value = item
}

async function upload(file: File, progress: (ratio: number) => void) {
  try {
    await api.upload(props.server, props.contentSlot, file, file.name, progress)
    toast.success(t('content.uploaded', { name: file.name }), props.live ? { description: t('content.nextRestart') } : undefined)
    emit('changed')
    await invalidate(props.server)
  } catch (error) {
    handleError(error)
  }
}

/** Contenu dont on modifie les valeurs. */
const editing = ref<ContentItem | null>(null)
const values = ref<Record<string, string>>({})
const valueErrors = ref<FieldErrors>({})

function editValues(item: ContentItem) {
  values.value = valuesTexts(props.fields, item.values)
  valueErrors.value = {}
  editing.value = item
}

async function saveValues() {
  const item = editing.value
  if (!item) return
  busy.value = item.id
  try {
    await api.setValues(props.server, props.contentSlot, item.id, valuesPayload(props.fields, values.value))
    toast.success(t('content.values.saved', { name: item.name }), props.live ? { description: t('content.nextRestart') } : undefined)
    editing.value = null
    emit('changed')
    await invalidate(props.server)
  } catch (error) {
    valueErrors.value = handleError(
      error,
      props.fields.map((field) => `values.${field.key}`),
    )
  } finally {
    busy.value = null
  }
}

async function confirmRemove() {
  const item = removing.value
  if (!item) return
  await run(item, () => api.remove(props.server, props.contentSlot, item.id, withOrphans.value && item.orphans.length > 0), 'content.removed')
  removing.value = null
}
</script>

<template>
  <div class="grid gap-2">
    <div v-if="manage && acceptsUpload" class="flex justify-end">
      <UploadButton :label="t('content.upload')" accept=".jar,.zip" :send="upload" />
    </div>
    <div v-if="isPending" class="grid gap-2"><Skeleton v-for="index in 3" :key="index" class="h-14" /></div>
    <Empty v-else-if="items.length === 0" :icon="Package">{{ t('content.empty') }}</Empty>
    <ul v-else class="grid gap-2">
      <li v-for="item in items" :key="item.id" class="flex flex-wrap items-center gap-3.5 rounded-xl border bg-card p-3 shadow-card">
        <img v-if="item.icon_url" :src="item.icon_url" alt="" class="size-[52px] rounded-[10px] bg-muted object-cover" loading="lazy" referrerpolicy="no-referrer">
        <div v-else class="flex size-[52px] items-center justify-center rounded-[10px] bg-muted"><Package class="size-5 text-muted-foreground" /></div>
        <div class="grid min-w-0 flex-1 gap-0.5">
          <p class="flex flex-wrap items-center gap-2 text-[15px] font-bold">
            <span :class="{ 'text-muted-foreground line-through': !item.enabled }">{{ item.name }}</span>
            <a
              v-if="item.page_url"
              :href="item.page_url"
              target="_blank"
              rel="noopener noreferrer"
              class="text-muted-foreground hover:text-foreground"
              :aria-label="t('content.pageOf', { name: item.name })"
              :title="t('content.pageOf', { name: item.name })"
            ><ExternalLink class="size-3.5" /></a>
            <span class="font-mono text-xs text-muted-foreground">{{ item.version_name }}</span>
            <Badge v-if="item.pending" variant="warning" class="font-normal">{{ t('content.pendingRestart') }}</Badge>
            <Badge v-if="item.state === 'installing'" variant="outline" class="font-normal">{{ t('content.state.installing') }}</Badge>
            <Badge v-else-if="item.state === 'failed'" variant="destructive" class="font-normal">{{ t('content.state.failed') }}</Badge>
            <Badge v-if="item.source === 'upload'" variant="secondary" class="font-normal">{{ t('content.source.upload') }}</Badge>
            <Badge v-if="item.kind === 'modpack'" variant="secondary" class="font-normal">{{ t('content.modpack', { count: filesOf(item).length }) }}</Badge>
          </p>
          <p v-if="item.failure" class="text-xs text-destructive">{{ item.failure }}</p>
          <p v-if="item.required_by.length" class="text-xs text-muted-foreground">{{ t('content.requiredBy', { names: names(item.required_by) }) }}</p>
          <p v-for="[key, text] in valuesSummary(item.values)" :key="key" class="text-xs text-muted-foreground">
            {{ `${fieldLabel(key)} : ` }}<span class="font-mono">{{ text }}</span>
          </p>
          <!-- Packs extraits de l'archive (add-ons Bedrock), rangés d'après leur description. -->
          <ul v-if="item.packages.length" class="grid gap-0.5 text-xs text-muted-foreground" :aria-label="t('content.packages')">
            <li v-for="pack in item.packages" :key="`${pack.dir}/${pack.name}`">
              {{ t('content.package', { title: pack.title ?? pack.name, version: pack.version ?? '—' }) }}
              <span class="font-mono">{{ `${pack.dir}/${pack.name}` }}</span>
            </li>
          </ul>
          <p class="text-xs text-muted-foreground">
            <span class="font-mono">{{ item.file_name }}</span>
            <template v-if="item.size_bytes != null">{{ ` · ${formatBytes(item.size_bytes, locale)}` }}</template>
          </p>
        </div>
        <Button
          v-if="item.update && manage && item.state === 'installed'"
          size="sm"
          variant="secondary"
          :disabled="busy === item.id"
          :aria-label="t('content.updateTo', { name: item.name, version: item.update.version_name })"
          @click="update(item)"
        >
          <ArrowUpCircle />{{ item.update.version_name }}
        </Button>
        <Button
          v-if="fields.length && manage && item.state === 'installed' && item.kind !== 'modpack'"
          size="icon"
          variant="ghost"
          class="size-8"
          :disabled="busy === item.id"
          :aria-label="t('content.values.edit', { name: item.name })"
          :title="t('content.values.edit', { name: item.name })"
          @click="editValues(item)"
        >
          <SlidersHorizontal />
        </Button>
        <Switch
          v-if="item.state === 'installed' && (item.kind === 'file' || item.kind === 'reference')"
          :model-value="item.enabled"
          :disabled="!manage || busy === item.id"
          :aria-label="t('content.toggle', { name: item.name })"
          @update:model-value="(value: boolean) => toggle(item, value)"
        />
        <Button
          v-if="manage"
          size="icon"
          variant="ghost"
          class="size-8"
          :disabled="busy === item.id || item.required_by.length > 0"
          :title="item.required_by.length ? t('content.cannotRemove') : undefined"
          :aria-label="t('content.remove', { name: item.name })"
          @click="askRemove(item)"
        >
          <Trash2 />
        </Button>
        <details v-if="item.kind === 'modpack' && filesOf(item).length" class="w-full text-xs text-muted-foreground">
          <summary class="cursor-pointer">{{ t('content.packFiles') }}</summary>
          <ul class="mt-1 grid gap-0.5 font-mono">
            <li v-for="file in filesOf(item)" :key="file.id">
              {{ file.file_name }}
              <span v-if="file.state === 'failed'" class="text-destructive">{{ ` · ${t('content.state.failed')}` }}</span>
            </li>
          </ul>
        </details>
      </li>
    </ul>

    <Dialog :open="editing !== null" @update:open="(open) => !open && (editing = null)">
      <DialogContent class="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{{ t('content.values.title', { name: editing?.name ?? '' }) }}</DialogTitle>
          <DialogDescription>{{ t('content.values.help') }}</DialogDescription>
        </DialogHeader>
        <ContentValuesForm v-model="values" :fields="fields" :label="fieldLabel" :help="fieldHelp" :errors="valueErrors" />
        <DialogFooter>
          <Button variant="ghost" @click="editing = null">{{ t('common.cancel') }}</Button>
          <Button :disabled="busy !== null" @click="saveValues">{{ t('common.save') }}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <AlertDialog :open="removing !== null" @update:open="(open) => !open && (removing = null)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('content.removeTitle', { name: removing?.name ?? '' }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('content.removeHelp') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <label v-if="removing?.orphans.length" class="flex items-start gap-2 text-sm">
          <input v-model="withOrphans" type="checkbox" class="mt-0.5 size-4 accent-primary">
          <span class="grid gap-1">
            <span>{{ t('content.removeOrphans', { names: names(removing.orphans) }) }}</span>
            <span class="text-xs text-muted-foreground">{{ t('content.removeOrphansHelp') }}</span>
          </span>
        </label>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="busy !== null" @click="confirmRemove">{{ t('content.removeConfirm') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
