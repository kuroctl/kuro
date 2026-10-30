<script setup lang="ts">
// Contenu d'un dossier dans l'arbre : chargé seulement quand le dossier est ouvert (le slot d'un
// `Folder` refermé n'est pas rendu), dossiers d'abord.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { File, Folder } from '@/shared/ui/file-tree'
import { Skeleton } from '@/shared/ui/skeleton'

import { useFileListing } from '../files.queries'
import { join } from '../paths'

defineOptions({ name: 'FileTreeNode' })

const props = defineProps<{ server: string; path: string }>()
const { t } = useI18n()
const { data: listing, isPending, error } = useFileListing(
  () => props.server,
  () => props.path,
)

const entries = computed(() =>
  [...(listing.value?.entries ?? [])]
    .filter((entry) => entry.kind === 'directory' || entry.kind === 'file')
    .sort((a, b) => Number(b.kind === 'directory') - Number(a.kind === 'directory') || a.name.localeCompare(b.name)),
)
</script>

<template>
  <Skeleton v-if="isPending" class="my-1 h-5 w-24" />
  <p v-else-if="error" class="px-2 py-1 text-xs text-destructive">{{ t('files.tree.unreadable') }}</p>
  <p v-else-if="!entries.length" class="py-1 pl-6 text-xs text-muted-foreground">{{ t('files.tree.empty') }}</p>
  <template v-for="entry in entries" v-else :key="entry.name">
    <Folder v-if="entry.kind === 'directory'" :id="join(path, entry.name)" :name="entry.name">
      <FileTreeNode :server="server" :path="join(path, entry.name)" />
    </Folder>
    <File v-else :id="join(path, entry.name)" :name="entry.name" />
  </template>
</template>
