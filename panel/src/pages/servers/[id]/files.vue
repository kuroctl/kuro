<script setup lang="ts">
// Fichiers d'un serveur : arbre des volumes à gauche, navigation dans un dossier (`?path=`) ou
// édition d'un fichier (`?file=`) à droite.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'

import FileBrowser from '@/features/files/components/FileBrowser.vue'
import FileEditor from '@/features/files/components/FileEditor.vue'
import ServerFileTree from '@/features/files/components/ServerFileTree.vue'
import { declaredFiles, parent } from '@/features/files/paths'
import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { usePrefab } from '@/features/prefabs/prefabs.queries'
import { useServer } from '@/features/servers/servers.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Card } from '@/shared/ui/card'

definePage({ meta: { subnav: 'server' } })

const { t } = useI18n()
const route = useRoute('/servers/[id]/files')
const router = useRouter()
const id = computed(() => route.params.id)
const { data: server } = useServer(id)
const { data: prefab } = usePrefab(
  () => server.value?.prefab.id,
  () => server.value?.prefab.version,
)
const locale = usePrefabLocale(() => server.value?.prefab)
const declared = computed(() => declaredFiles(prefab.value?.manifest, server.value?.settings ?? {}))

const query = (name: string) => {
  const value = route.query[name]
  return typeof value === 'string' && value.length > 0 ? value : undefined
}
const directory = computed(() => query('path') ?? '/')
const file = computed(() => query('file'))
const opened = computed(() => declared.value.find((entry) => entry.path === file.value))

function navigate(path: string) {
  void router.push({ query: path === '/' ? {} : { path } })
}

function open(path: string) {
  void router.push({ query: { file: path } })
}

function close() {
  if (file.value) navigate(parent(file.value))
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <PageHeader :title="t('files.title')" />
    <div class="grid min-h-0 flex-1 gap-6 p-6 lg:grid-cols-[17rem_minmax(0,1fr)]">
      <Card class="hidden max-h-[calc(100vh-10rem)] gap-0 self-start overflow-hidden py-0 lg:sticky lg:top-6 lg:flex">
        <ServerFileTree
          :server="id"
          :current="file ?? directory"
          :directory="file ? parent(file) : directory"
          @navigate="navigate"
          @open="open"
        />
      </Card>
      <div class="flex min-h-0 min-w-0 flex-col">
        <Card v-if="file" class="flex min-h-[32rem] flex-1 flex-col gap-0 overflow-hidden py-0">
          <FileEditor
            :server="id"
            :path="file"
            :format="opened?.format"
            :description="opened ? locale.fileDescription(opened.key) : undefined"
            @close="close"
          />
        </Card>
        <FileBrowser v-else :server="id" :path="directory" :declared="declared" :locale="locale" @navigate="navigate" @open="open" />
      </div>
    </div>
  </div>
</template>
