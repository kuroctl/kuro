<script setup lang="ts">
// Sous-page d'un groupe de réglages, générée depuis le prefab du serveur.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { usePrefabSettings } from '@/features/prefabs/composables/usePrefabSettings'
import { usePrefab } from '@/features/prefabs/prefabs.queries'
import ServerSettingsForm from '@/features/servers/components/ServerSettingsForm.vue'
import { useServer } from '@/features/servers/servers.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Skeleton } from '@/shared/ui/skeleton'

definePage({ meta: { subnav: 'server' } })

const { t } = useI18n()
const route = useRoute('/servers/[id]/settings/[section]/[group]')
const { data: server } = useServer(() => route.params.id)
const { data: prefab } = usePrefab(
  () => server.value?.prefab.id,
  () => server.value?.prefab.version,
)
const settings = usePrefabSettings(() => prefab.value?.manifest)
const locale = usePrefabLocale(() => server.value?.prefab)
const group = computed(() => settings.group(route.params.section, route.params.group))
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="group ? locale.groupTitle(group.key) : ''">
      <span class="text-xs text-muted-foreground">{{ t(`servers.nav.${route.params.section}`) }}</span>
    </PageHeader>
    <div class="max-w-2xl p-6">
      <Skeleton v-if="!server || !prefab" class="h-64" />
      <p v-else-if="!group" class="text-sm text-muted-foreground">{{ t('servers.settings.unknownGroup') }}</p>
      <!-- Clé : un formulaire neuf par groupe et par version du serveur. -->
      <ServerSettingsForm
        v-else
        :key="`${group.key}-${server.updated_at}`"
        :server="server"
        :fields="group.fields"
        :locale="locale"
        :needs-restart="settings.needsRestart"
      />
    </div>
  </div>
</template>
