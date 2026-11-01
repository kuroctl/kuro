<script setup lang="ts">
// Fichier ouvert : chargement, enregistrement protégé contre l'écrasement (`if_sha256`), et
// confirmation avant de quitter des modifications non enregistrées.
import { RotateCw, Save, X } from '@lucide/vue'
import { computed, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { onBeforeRouteLeave, onBeforeRouteUpdate } from 'vue-router'
import { toast } from 'vue-sonner'

import { problemCode, useApiError } from '@/shared/api/useApiError'
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Skeleton } from '@/shared/ui/skeleton'

import { useFilesApi } from '../files.api'
import { useFileContent } from '../files.queries'
import { formatOf } from '../language'
import { basename } from '../paths'
import CodeEditor from './CodeEditor.vue'
import FileHistory from './FileHistory.vue'

const props = defineProps<{ server: string; path: string; format?: string; description?: string }>()
const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()
const { write } = useFilesApi()
const { handleError, message } = useApiError()
const { data: file, error, refetch, isFetching } = useFileContent(
  () => props.server,
  () => props.path,
)
const editor = useTemplateRef<InstanceType<typeof CodeEditor>>('editor')

const dirty = ref(false)
const saving = ref(false)
const conflict = ref(false)
const leaving = ref<((leave: boolean) => void) | null>(null)
// Empreinte de la version sur laquelle on travaille : elle change à chaque enregistrement.
const sha256 = ref<string>()
const effectiveFormat = computed(() => formatOf(props.path, props.format))

async function save(overwrite = false) {
  if (!editor.value || saving.value) return
  const content = editor.value.content()
  saving.value = true
  try {
    await write(props.server, {
      path: props.path,
      content,
      if_sha256: overwrite ? null : (sha256.value ?? file.value?.sha256 ?? null),
    })
    const reloaded = await refetch()
    sha256.value = reloaded.data?.sha256
    editor.value.markSaved(content)
    conflict.value = false
    toast.success(t('files.editor.saved'))
  } catch (caught) {
    if (problemCode(caught) === 'file_conflict') conflict.value = true
    else handleError(caught)
  } finally {
    saving.value = false
  }
}

async function reload() {
  const reloaded = await refetch()
  if (reloaded.data) {
    sha256.value = reloaded.data.sha256
    editor.value?.reset(reloaded.data.content)
  }
  conflict.value = false
}

/** Quitter le fichier : confirmation si des modifications ne sont pas enregistrées. */
function confirmLeave(): Promise<boolean> | boolean {
  if (!dirty.value) return true
  return new Promise((resolve) => (leaving.value = resolve))
}

function answerLeave(leave: boolean) {
  leaving.value?.(leave)
  leaving.value = null
}

onBeforeRouteLeave(confirmLeave)
onBeforeRouteUpdate(confirmLeave)
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="flex flex-wrap items-center gap-2 border-b px-4 py-2">
      <div class="min-w-0 flex-1">
        <p class="truncate font-mono text-sm">{{ path }}</p>
        <p v-if="description" class="truncate text-xs text-muted-foreground">{{ description }}</p>
      </div>
      <Badge v-if="dirty" variant="secondary">{{ t('files.editor.unsaved') }}</Badge>
      <FileHistory
        :server="server"
        :path="path"
        :format="effectiveFormat"
        :current-content="() => editor?.content() ?? file?.content ?? ''"
        :dirty="dirty"
        @restored="reload"
      />
      <Button size="sm" variant="ghost" :disabled="isFetching || saving" @click="reload">
        <RotateCw />{{ t('files.editor.reload') }}
      </Button>
      <Button size="sm" :disabled="!file || saving || !dirty" @click="save()">
        <Save />{{ t('common.save') }}
      </Button>
      <Button size="icon" variant="ghost" :aria-label="t('files.editor.close')" @click="emit('close')">
        <X />
      </Button>
    </div>

    <p v-if="error" role="alert" class="m-4 rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">
      {{ message(error) }}
    </p>
    <div v-else-if="!file" class="grid gap-2 p-4"><Skeleton v-for="index in 6" :key="index" class="h-5" /></div>
    <CodeEditor
      v-else
      ref="editor"
      :key="path"
      :initial="file.content"
      :format="effectiveFormat"
      :label="t('files.editor.label', { name: basename(path) })"
      @change="dirty = $event"
      @save="save()"
    />

    <AlertDialog :open="conflict" @update:open="(open) => !open && (conflict = false)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('files.conflict.title') }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('files.conflict.help') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button variant="outline" @click="reload">{{ t('files.conflict.reload') }}</Button>
          <Button variant="destructive" :disabled="saving" @click="save(true)">{{ t('files.conflict.overwrite') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

    <AlertDialog :open="leaving !== null" @update:open="(open) => !open && answerLeave(false)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('files.leave.title') }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('files.leave.help') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button variant="outline" @click="answerLeave(false)">{{ t('files.leave.stay') }}</Button>
          <Button variant="destructive" @click="answerLeave(true)">{{ t('files.leave.discard') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
