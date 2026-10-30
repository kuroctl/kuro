<script setup lang="ts">
// Aperçu du formulaire de réglages du brouillon, dans la langue du panel (repli sur l'anglais),
// depuis le dernier état valide du manifest.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { useMessagesLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { usePrefabSettings } from '@/features/prefabs/composables/usePrefabSettings'
import type { Manifest } from '@/features/prefabs/manifest'
import type { PrefabMessages } from '@/features/prefabs/prefabs.api'

import DraftPreviewGroup from './DraftPreviewGroup.vue'

const props = defineProps<{ manifest: Manifest; messages: Record<string, PrefabMessages> }>()

const { t } = useI18n()
const locale = useMessagesLocale(
  () => props.messages,
  () => props.manifest.id,
)
const { sections } = usePrefabSettings(() => props.manifest)
const description = computed(() => locale.text('prefab.description'))
</script>

<template>
  <div class="grid gap-6">
    <div>
      <p class="text-lg font-bold">{{ locale.prefabName() }}</p>
      <p v-if="description" class="text-sm text-muted-foreground">{{ description }}</p>
    </div>
    <p v-if="!sections.length" class="text-sm text-muted-foreground">{{ t('servers.create.noSettings') }}</p>
    <section v-for="section in sections" :key="section.key" class="grid gap-4">
      <h3 class="text-xs font-semibold tracking-wide text-muted-foreground uppercase">{{ t(`servers.nav.${section.key}`) }}</h3>
      <div v-for="group in section.groups" :key="group.key" class="grid gap-4 rounded-xl border bg-card p-4 shadow-card">
        <p class="text-sm font-semibold">{{ locale.groupTitle(group.key) }}</p>
        <!-- Un formulaire neuf quand les champs changent. -->
        <DraftPreviewGroup :key="JSON.stringify(group.fields)" :fields="group.fields" :locale="locale" />
      </div>
    </section>
  </div>
</template>
