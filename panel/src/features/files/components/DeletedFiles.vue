<script setup lang="ts">
// Fichiers supprimés du dossier dont l'historique garde une version (ADR-0049), à remettre en
// un clic. Rien ne s'affiche s'il n'y en a pas.
import { ArchiveRestore, ChevronDown } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { useApiError } from '@/shared/api/useApiError'
import { formatBytes } from '@/shared/lib/format'
import { Button } from '@/shared/ui/button'
import { Card } from '@/shared/ui/card'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/shared/ui/collapsible'

import { useFilesApi } from '../files.api'
import { useDeletedFiles, useInvalidateFiles } from '../files.queries'
import { basename } from '../paths'

const props = defineProps<{ server: string; path: string }>()

const { t, locale } = useI18n()
const { can } = usePermissions()
const { handleError } = useApiError()
const invalidate = useInvalidateFiles()
const { data } = useDeletedFiles(
  () => props.server,
  () => props.path,
)
const files = computed(() => data.value?.files ?? [])
const restoring = ref<string>()
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }))

async function restore(path: string, version: string) {
  restoring.value = path
  try {
    await useFilesApi().restore(props.server, path, version)
    await invalidate(props.server)
    toast.success(t('files.deleted.restored', { name: basename(path) }))
  } catch (caught) {
    handleError(caught)
  } finally {
    restoring.value = undefined
  }
}
</script>

<template>
  <Collapsible v-if="files.length > 0">
    <Card class="gap-0 py-0">
      <CollapsibleTrigger as-child>
        <button type="button" class="group flex w-full items-center gap-2 px-4 py-2.5 text-left text-sm hover:bg-accent/50">
          <ArchiveRestore class="size-4 text-muted-foreground" />
          <span class="flex-1">{{ t('files.deleted.title', { count: files.length }, files.length) }}</span>
          <ChevronDown class="size-4 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <ul class="divide-y border-t">
          <li v-for="file in files" :key="file.path" class="flex items-center gap-3 px-4 py-2">
            <div class="grid min-w-0 flex-1 gap-0.5">
              <span class="truncate font-mono text-sm">{{ basename(file.path) }}</span>
              <span class="text-xs text-muted-foreground">
                {{ t('files.deleted.when', { date: dates.format(new Date(file.version.at_ms)), size: formatBytes(file.version.size, locale) }) }}
              </span>
            </div>
            <Button
              v-if="can('servers.files.write')"
              size="sm"
              variant="outline"
              :disabled="restoring === file.path"
              @click="restore(file.path, file.version.id)"
            >
              {{ t('files.deleted.restore') }}
            </Button>
          </li>
        </ul>
      </CollapsibleContent>
    </Card>
  </Collapsible>
</template>
