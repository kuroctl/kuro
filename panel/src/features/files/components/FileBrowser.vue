<script setup lang="ts">
// Dossier d'un serveur : fichiers déclarés du prefab en tête, puis le contenu du dossier.
// Création, renommage et suppression seulement sous un volume (les deux premiers niveaux,
// service et volume, sont des points de montage).
import { File, FilePlus, FileText, Folder, FolderPlus, Link2, MoreHorizontal, Pencil, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import type { PrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { useApiError } from '@/shared/api/useApiError'
import { formatBytes } from '@/shared/lib/format'
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Badge } from '@/shared/ui/badge'
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from '@/shared/ui/breadcrumb'
import { Button } from '@/shared/ui/button'
import { Card } from '@/shared/ui/card'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/shared/ui/dropdown-menu'
import { Skeleton } from '@/shared/ui/skeleton'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

import { type FileEntry, useFilesApi } from '../files.api'
import { useFileListing, useInvalidateFiles } from '../files.queries'
import { type DeclaredFile, isEditable, join, segments } from '../paths'
import NameDialog from './NameDialog.vue'
import UploadButton from './UploadButton.vue'

/** Au-delà, l'API refuse d'ouvrir le fichier dans l'éditeur. */
const MAX_TEXT_BYTES = 512 * 1024

const props = defineProps<{ server: string; path: string; declared: DeclaredFile[]; locale: PrefabLocale }>()
const emit = defineEmits<{ navigate: [path: string]; open: [path: string] }>()

const { t, locale: language } = useI18n()
const api = useFilesApi()
const { handleError, message } = useApiError()
const invalidate = useInvalidateFiles()
const { data: listing, error } = useFileListing(
  () => props.server,
  () => props.path,
)

const editable = computed(() => isEditable(props.path))
const crumbs = computed(() =>
  segments(props.path).map((name, index, all) => ({ name, path: `/${all.slice(0, index + 1).join('/')}` })),
)
const dates = computed(() => new Intl.DateTimeFormat(language.value, { dateStyle: 'short', timeStyle: 'short' }))

type Dialog = { kind: 'folder' } | { kind: 'file' } | { kind: 'rename'; entry: FileEntry }
const dialog = ref<Dialog | null>(null)
const deleting = ref<FileEntry | null>(null)
const pending = ref(false)

function openable(entry: FileEntry): boolean {
  return entry.kind === 'file' && entry.size <= MAX_TEXT_BYTES
}

function activate(entry: FileEntry) {
  const path = join(props.path, entry.name)
  if (entry.kind === 'directory') emit('navigate', path)
  else if (openable(entry)) emit('open', path)
}

/** Retourne un message à afficher sous la saisie, ou rien si l'opération a réussi. */
async function submitName(name: string): Promise<string | undefined> {
  const current = dialog.value
  if (!current) return undefined
  if (current.kind === 'folder') {
    await api.mkdir(props.server, join(props.path, name))
  } else if (current.kind === 'file') {
    // L'écriture remplacerait un fichier existant : la création ne doit jamais écraser.
    if (listing.value?.entries.some((entry) => entry.name === name)) return t('errors.file_exists')
    await api.write(props.server, { path: join(props.path, name), content: '', if_sha256: null })
  } else {
    await api.rename(props.server, join(props.path, current.entry.name), join(props.path, name))
  }
  await invalidate(props.server)
  if (current.kind === 'file') emit('open', join(props.path, name))
  return undefined
}

/** Envoi dans le dossier courant ; un fichier du même nom est remplacé. */
async function upload(file: File, progress: (ratio: number) => void) {
  try {
    await api.upload(props.server, join(props.path, file.name), file, progress)
    toast.success(t('files.upload.done', { name: file.name }))
    await invalidate(props.server)
  } catch (caught) {
    handleError(caught)
  }
}

async function confirmDelete() {
  const entry = deleting.value
  if (!entry) return
  pending.value = true
  try {
    await api.remove(props.server, join(props.path, entry.name))
    toast.success(t('files.delete.done', { name: entry.name }))
    await invalidate(props.server)
  } catch (caught) {
    handleError(caught)
  } finally {
    pending.value = false
    deleting.value = null
  }
}

const dialogTexts = computed(() => {
  const current = dialog.value
  if (!current) return { title: '', description: '', action: '' }
  if (current.kind === 'rename') {
    return { title: t('files.rename.title', { name: current.entry.name }), description: t('files.rename.help'), action: t('files.rename.action') }
  }
  return {
    title: t(`files.${current.kind === 'folder' ? 'newFolder' : 'newFile'}.title`),
    description: t('files.newHelp', { path: props.path }),
    action: t('files.create'),
  }
})
</script>

<template>
  <div data-slot="file-browser" class="grid gap-4">
    <Card v-if="path === '/' && declared.length > 0" class="gap-0 py-0">
      <p class="border-b px-4 py-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{{ t('files.declared') }}</p>
      <ul class="divide-y">
        <li v-for="file in declared" :key="file.key">
          <button type="button" class="flex w-full items-start gap-3 px-4 py-2.5 text-left hover:bg-accent/50" @click="emit('open', file.path)">
            <FileText class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
            <span class="min-w-0">
              <span class="flex min-w-0 items-center gap-2">
                <span class="block truncate font-mono text-sm">{{ file.path }}</span>
                <Badge v-if="file.generated" variant="secondary" class="shrink-0 font-normal">{{ t('files.generated') }}</Badge>
              </span>
              <span v-if="file.generated" class="block text-xs text-muted-foreground">{{ t('files.registerHelp') }}</span>
              <span v-else-if="locale.fileDescription(file.key)" class="block text-xs text-muted-foreground">{{ locale.fileDescription(file.key) }}</span>
            </span>
          </button>
        </li>
      </ul>
    </Card>

    <div class="flex flex-wrap items-center gap-2">
      <Breadcrumb class="min-w-0 flex-1">
        <BreadcrumbList>
          <BreadcrumbItem>
            <BreadcrumbPage v-if="crumbs.length === 0">{{ t('files.root') }}</BreadcrumbPage>
            <BreadcrumbLink v-else as="button" type="button" @click="emit('navigate', '/')">{{ t('files.root') }}</BreadcrumbLink>
          </BreadcrumbItem>
          <template v-for="(crumb, index) in crumbs" :key="crumb.path">
            <BreadcrumbSeparator />
            <BreadcrumbItem>
              <BreadcrumbPage v-if="index === crumbs.length - 1" class="font-mono">{{ crumb.name }}</BreadcrumbPage>
              <BreadcrumbLink v-else as="button" type="button" class="font-mono" @click="emit('navigate', crumb.path)">{{ crumb.name }}</BreadcrumbLink>
            </BreadcrumbItem>
          </template>
        </BreadcrumbList>
      </Breadcrumb>
      <template v-if="editable">
        <Button size="sm" variant="outline" @click="dialog = { kind: 'folder' }"><FolderPlus />{{ t('files.newFolder.action') }}</Button>
        <Button size="sm" variant="outline" @click="dialog = { kind: 'file' }"><FilePlus />{{ t('files.newFile.action') }}</Button>
        <UploadButton :label="t('files.upload.action')" :send="upload" />
      </template>
    </div>

    <p v-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
    <div v-else-if="!listing" class="grid gap-2"><Skeleton v-for="index in 5" :key="index" class="h-9" /></div>
    <p v-else-if="listing.entries.length === 0" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t('files.empty') }}</p>
    <Table v-else>
      <TableHeader>
        <TableRow>
          <TableHead>{{ t('files.columns.name') }}</TableHead>
          <TableHead class="w-28 text-right">{{ t('files.columns.size') }}</TableHead>
          <TableHead class="w-40">{{ t('files.columns.modified') }}</TableHead>
          <TableHead class="w-10" />
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow v-for="entry in listing.entries" :key="entry.name">
          <TableCell>
            <button
              v-if="entry.kind === 'directory' || openable(entry)"
              type="button"
              class="flex items-center gap-2 text-left hover:underline"
              @click="activate(entry)"
            >
              <Folder v-if="entry.kind === 'directory'" class="size-4 text-muted-foreground" />
              <File v-else class="size-4 text-muted-foreground" />
              <span class="font-mono text-sm">{{ entry.name }}</span>
            </button>
            <span v-else class="flex items-center gap-2 text-muted-foreground">
              <Link2 v-if="entry.kind === 'symlink'" class="size-4" />
              <File v-else class="size-4" />
              <span class="font-mono text-sm">{{ entry.name }}</span>
              <Badge variant="outline" class="text-[10px]">
                {{ entry.kind === 'symlink' ? t('files.kind.symlink') : entry.kind === 'file' ? t('files.kind.tooLarge') : t('files.kind.other') }}
              </Badge>
            </span>
          </TableCell>
          <TableCell class="text-right tabular-nums text-muted-foreground">{{ entry.kind === 'file' ? formatBytes(entry.size, language) : '' }}</TableCell>
          <TableCell class="text-muted-foreground">{{ entry.modified_ms > 0 ? dates.format(entry.modified_ms) : '' }}</TableCell>
          <TableCell>
            <DropdownMenu v-if="editable">
              <DropdownMenuTrigger as-child>
                <Button size="icon" variant="ghost" class="size-8" :aria-label="t('files.actions', { name: entry.name })"><MoreHorizontal /></Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem @select="dialog = { kind: 'rename', entry }"><Pencil />{{ t('files.rename.action') }}</DropdownMenuItem>
                <DropdownMenuItem variant="destructive" @select="deleting = entry"><Trash2 />{{ t('files.delete.action') }}</DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>

    <NameDialog
      :open="dialog !== null"
      :title="dialogTexts.title"
      :description="dialogTexts.description"
      :action="dialogTexts.action"
      :initial="dialog?.kind === 'rename' ? dialog.entry.name : ''"
      :submit="submitName"
      @update:open="(open) => !open && (dialog = null)"
    />

    <AlertDialog :open="deleting !== null" @update:open="(open) => !open && (deleting = null)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('files.delete.title', { name: deleting?.name ?? '' }) }}</AlertDialogTitle>
          <AlertDialogDescription>
            {{ deleting?.kind === 'directory' ? t('files.delete.directoryHelp') : t('files.delete.help') }}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button variant="outline" @click="deleting = null">{{ t('common.cancel') }}</Button>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="confirmDelete">{{ t('files.delete.confirm') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
