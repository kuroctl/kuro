<script setup lang="ts">
// Historique d'un fichier (ADR-0049) : versions gardées avant chaque écriture de Kuro (panel,
// SFTP, restauration, réécriture au démarrage), comparaison avec la version actuelle, et
// restauration, elle-même annulable puisque la version remplacée est gardée à son tour.
import { History, RotateCcw } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { problemCode, useApiError } from '@/shared/api/useApiError'
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
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from '@/shared/ui/sheet'
import { Skeleton } from '@/shared/ui/skeleton'

import { type FileVersion, useFilesApi } from '../files.api'
import { useFileHistory, useFileVersion, useInvalidateFiles } from '../files.queries'
import VersionDiff from './VersionDiff.vue'

const props = defineProps<{ server: string; path: string; format?: string; currentContent: () => string; dirty: boolean }>()
const emit = defineEmits<{ restored: [] }>()

const { t, locale } = useI18n()
const { can, profile } = usePermissions()
const { handleError, message } = useApiError()
const invalidate = useInvalidateFiles()
const open = ref(false)
const selected = ref<FileVersion>()
const confirming = ref(false)
const restoring = ref(false)
// Contenu actuel figé à l'ouverture d'une version : la comparaison ne bouge pas pendant qu'on la lit.
const current = ref('')

const { data: history, isPending, error } = useFileHistory(
  () => props.server,
  () => props.path,
  open,
)
const version = useFileVersion(
  () => props.server,
  () => props.path,
  () => selected.value?.id,
)
const versions = computed(() => history.value?.versions ?? [])
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }))

watch(open, (opened) => {
  if (!opened) selected.value = undefined
})

function select(entry: FileVersion) {
  current.value = props.currentContent()
  selected.value = entry
}

function author(entry: FileVersion): string {
  if (entry.author_kind === 'system') return t('files.history.author.system')
  if (entry.author_id && entry.author_id === profile.value?.user.id) return t('files.history.author.you')
  if (entry.author_kind === 'agent') return t('files.history.author.agent')
  if (entry.author_kind) return t('files.history.author.admin')
  return t('files.history.author.unknown')
}

function reason(entry: FileVersion): string {
  const known = ['write', 'delete', 'restore', 'start'].includes(entry.reason)
  return t(`files.history.reason.${known ? entry.reason : 'other'}`)
}

function channel(entry: FileVersion): string {
  const known = ['web', 'sftp', 'kuro'].includes(entry.channel)
  return t(`files.history.channel.${known ? entry.channel : 'other'}`)
}

async function restore() {
  const entry = selected.value
  if (!entry) return
  restoring.value = true
  try {
    await useFilesApi().restore(props.server, props.path, entry.id)
    await invalidate(props.server)
    toast.success(t('files.history.restored', { date: dates.value.format(new Date(entry.at_ms)) }))
    confirming.value = false
    open.value = false
    emit('restored')
  } catch (caught) {
    handleError(caught)
  } finally {
    restoring.value = false
  }
}
</script>

<template>
  <Button size="sm" variant="ghost" @click="open = true"><History />{{ t('files.history.action') }}</Button>

  <Sheet v-model:open="open">
    <SheetContent side="right" class="flex w-full flex-col gap-0 sm:max-w-5xl">
      <SheetHeader class="border-b">
        <SheetTitle>{{ t('files.history.title') }}</SheetTitle>
        <SheetDescription class="font-mono">{{ path }}</SheetDescription>
      </SheetHeader>
      <div class="grid min-h-0 flex-1 gap-0 md:grid-cols-[18rem_minmax(0,1fr)]">
        <div class="min-h-0 overflow-y-auto border-b md:border-r md:border-b-0">
          <div v-if="isPending" class="grid gap-2 p-4"><Skeleton v-for="index in 4" :key="index" class="h-12" /></div>
          <p v-else-if="error" role="alert" class="p-4 text-sm text-destructive">{{ message(error) }}</p>
          <p v-else-if="versions.length === 0" class="p-4 text-sm text-muted-foreground">{{ t('files.history.empty') }}</p>
          <ul v-else class="divide-y">
            <li v-for="entry in versions" :key="entry.id">
              <button
                type="button"
                class="grid w-full gap-1 px-4 py-3 text-left hover:bg-accent/50"
                :class="{ 'bg-accent': selected?.id === entry.id }"
                :aria-pressed="selected?.id === entry.id"
                @click="select(entry)"
              >
                <span class="text-sm font-medium">{{ dates.format(new Date(entry.at_ms)) }}</span>
                <span class="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                  <Badge variant="secondary" class="font-normal">{{ reason(entry) }}</Badge>
                  <span>{{ channel(entry) }}</span>
                  <span>{{ author(entry) }}</span>
                </span>
              </button>
            </li>
          </ul>
          <p class="p-4 text-xs text-muted-foreground">{{ t('files.history.limits') }}</p>
        </div>

        <div class="flex min-h-0 flex-col gap-3 p-4">
          <p v-if="!selected" class="text-sm text-muted-foreground">{{ t('files.history.pick') }}</p>
          <template v-else>
            <div class="flex flex-wrap items-center gap-2">
              <p class="min-w-0 flex-1 text-sm">
                {{ t('files.history.comparing', { date: dates.format(new Date(selected.at_ms)) }) }}
              </p>
              <Button v-if="can('servers.files.write')" size="sm" @click="confirming = true">
                <RotateCcw />{{ t('files.history.restore') }}
              </Button>
            </div>
            <Skeleton v-if="version.isPending.value" class="h-64" />
            <p
              v-else-if="version.error.value && problemCode(version.error.value) === 'file_not_text'"
              class="rounded-lg border px-4 py-3 text-sm text-muted-foreground"
            >
              {{ t('files.history.binary') }}
            </p>
            <p v-else-if="version.error.value" role="alert" class="text-sm text-destructive">{{ message(version.error.value) }}</p>
            <VersionDiff
              v-else-if="version.data.value"
              class="min-h-0 flex-1"
              :before="version.data.value.content"
              :after="current"
              :format="format"
              :before-label="t('files.history.before')"
              :after-label="t('files.history.after')"
            />
          </template>
        </div>
      </div>
    </SheetContent>
  </Sheet>

  <AlertDialog :open="confirming" @update:open="(value) => !value && (confirming = false)">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>{{ t('files.history.confirmTitle') }}</AlertDialogTitle>
        <AlertDialogDescription>
          {{ t('files.history.confirmHelp') }}
          <template v-if="dirty"> {{ t('files.history.confirmDirty') }}</template>
        </AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
        <Button :disabled="restoring" @click="restore">{{ t('files.history.restore') }}</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
