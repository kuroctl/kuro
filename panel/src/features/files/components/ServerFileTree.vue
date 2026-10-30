<script setup lang="ts">
// Arbre des volumes d'un serveur, à côté de la liste ou de l'éditeur : un dossier choisi
// s'affiche dans la liste, un fichier s'ouvre dans l'éditeur. Les dossiers menant à
// l'emplacement courant sont ouverts.
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { Tree, type TreeItemKind } from '@/shared/ui/file-tree'

import { segments } from '../paths'
import FileTreeNode from './FileTreeNode.vue'

const props = defineProps<{ server: string; current: string; directory: string }>()
const emit = defineEmits<{ navigate: [path: string]; open: [path: string] }>()
const { t } = useI18n()

/** Dossiers de `/` jusqu'à `path` inclus. */
const ancestors = (path: string) => segments(path).map((_, index, all) => `/${all.slice(0, index + 1).join('/')}`)

const expanded = ref<string[]>(ancestors(props.directory))
watch(
  () => props.directory,
  (directory) => {
    expanded.value = [...new Set([...expanded.value, ...ancestors(directory)])]
  },
)

function select(path: string, kind: TreeItemKind) {
  if (kind === 'folder') emit('navigate', path)
  else emit('open', path)
}
</script>

<template>
  <nav :aria-label="t('files.tree.label')" class="flex min-h-0 flex-col">
    <p class="px-4 pt-4 pb-2 text-xs font-semibold tracking-wide text-muted-foreground uppercase">{{ t('files.tree.label') }}</p>
    <Tree v-model:expanded="expanded" :selected="current" class="pb-3" @select="select">
      <FileTreeNode :server="server" path="/" />
    </Tree>
  </nav>
</template>
