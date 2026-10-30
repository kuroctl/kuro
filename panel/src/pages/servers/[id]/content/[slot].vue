<script setup lang="ts">
// Contenu d'un emplacement du serveur (ADR-0020, ADR-0029) : installés et recherche, depuis les
// sources de l'emplacement. Un loader vanilla ou une version du jeu non figée empêchent d'ajouter
// du contenu : la page l'explique et propose de figer la version. En mode `reference`, le jeu
// télécharge lui-même ses mods : chaque changement redéploie le serveur.
import { Info, TriangleAlert } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import ContentBrowser from '@/features/content/components/ContentBrowser.vue'
import LoaderIcon from '@/features/content/components/LoaderIcon.vue'
import InstalledContent from '@/features/content/components/InstalledContent.vue'
import PendingRestartBanner from '@/features/servers/components/PendingRestartBanner.vue'
import ModpackBrowser from '@/features/content/components/ModpackBrowser.vue'
import { useContentSlots, useInstalledContent, useInvalidateContent } from '@/features/content/content.queries'
import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { useServersApi } from '@/features/servers/servers.api'
import { useInvalidateServers, useServer } from '@/features/servers/servers.queries'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Alert } from '@/shared/ui/alert'
import { Button } from '@/shared/ui/button'
import { Skeleton } from '@/shared/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/ui/tabs'

definePage({ meta: { subnav: 'server' } })

const { t } = useI18n()
const route = useRoute('/servers/[id]/content/[slot]')
const id = computed(() => route.params.id)
const slotKey = computed(() => route.params.slot)
const { data: server } = useServer(id)
const { can } = usePermissions()
const { data: slots, isPending } = useContentSlots(id, () => can('servers.content.read'))
const locale = usePrefabLocale(() => server.value?.prefab)
const { updateSettings } = useServersApi()
const { handleError } = useApiError()
const invalidateServers = useInvalidateServers()
const invalidateContent = useInvalidateContent()

const slot = computed(() => slots.value?.find((s) => s.slot === slotKey.value))
const { data: installed } = useInstalledContent(id, slotKey)
const hasPack = computed(() => installed.value?.items.some((item) => item.kind === 'modpack') ?? false)
const tab = ref('installed')
/** Sources déclarées par le prefab que Kuro ne gère pas encore, nommées dans l'alerte. */
const unsupported = computed(() =>
  (slot.value?.sources ?? [])
    .filter((s) => s.blocked === 'content_source_unsupported')
    .map((s) => t(`content.sources.${s.provider}`))
    .join(', '),
)
/** Parcourir reste ouvert sans clé d'API : la page explique où la régler. */
const browseBlocked = computed(() => !!slot.value?.blocked && slot.value.blocked !== 'content_source_not_configured')
const changed = ref(false)
/** Serveur en marche : un changement de contenu attend son prochain redémarrage. */
const live = computed(() => server.value?.state === 'running' || server.value?.state === 'starting')
// Un contenu change d'état (installé, en attente) : le bandeau du serveur suit.
watch(
  () => (installed.value?.items ?? []).map((item) => `${item.id}:${item.state}:${item.pending}`).join(','),
  (now: string, before: string | undefined) => {
    if (before !== undefined && now !== before) void invalidateServers()
  },
)
const pinning = ref(false)

async function pin() {
  const version = slot.value?.latest_game_version
  if (!version) return
  pinning.value = true
  try {
    await updateSettings(id.value, { version })
    toast.success(t('content.pinned', { version }))
    await Promise.all([invalidateServers(), invalidateContent(id.value)])
  } catch (error) {
    handleError(error)
  } finally {
    pinning.value = false
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="locale.contentTitle(slotKey)" />
    <div class="grid max-w-4xl gap-4 p-6">
      <p v-if="!can('servers.content.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t('content.forbidden') }}</p>
      <Skeleton v-else-if="isPending || !server" class="h-40" />
      <p v-else-if="!slot" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">{{ t('content.noSlot') }}</p>
      <template v-else>
        <p v-if="slot.loader || slot.game_version" class="flex items-center gap-2 text-sm text-muted-foreground">
          <LoaderIcon :name="slot.loader" />
          {{ t('content.compat', { loader: slot.loader ?? '—', version: slot.game_version ?? '—' }) }}
        </p>
        <Alert v-if="slot.blocked === 'content_not_supported'" variant="warning"><TriangleAlert />{{ t('content.notSupported') }}</Alert>
        <Alert v-else-if="slot.blocked === 'content_source_unsupported'" variant="warning">
          <TriangleAlert />{{ t('content.sourceUnsupported', { sources: unsupported }) }}
        </Alert>
        <Alert v-else-if="slot.blocked === 'game_version_not_pinned'" variant="warning" class="flex-wrap items-center">
          <TriangleAlert />
          <span class="flex-1">{{ t('content.notPinned') }}</span>
          <Button v-if="slot.latest_game_version && can('servers.update')" size="sm" :disabled="pinning" @click="pin">
            {{ t('content.pin', { version: slot.latest_game_version }) }}
          </Button>
        </Alert>
        <Alert v-if="slot.mode === 'reference'" variant="info"><Info />{{ t('content.reference') }}</Alert>
        <PendingRestartBanner :server="server" />

        <Tabs v-model="tab">
          <TabsList>
            <TabsTrigger value="installed">{{ t('content.tabs.installed') }}</TabsTrigger>
            <TabsTrigger value="browse" :disabled="browseBlocked">{{ t('content.tabs.browse') }}</TabsTrigger>
            <TabsTrigger v-if="slot.kind === 'mods' && slot.mode === 'download' && slot.sources.some((s) => s.provider === 'modrinth') && slot.blocked !== 'content_not_supported'" value="modpacks">{{ t('content.tabs.modpacks') }}</TabsTrigger>
          </TabsList>
          <TabsContent value="installed" class="mt-4">
            <InstalledContent
              :server="id"
              :content-slot="slotKey"
              :suspended="!!server.suspended_at"
              :accepts-upload="slot.upload"
              :fields="slot.fields"
              :field-label="(field: string) => locale.contentFieldLabel(slotKey, field)"
              :field-help="(field: string) => locale.contentFieldHelp(slotKey, field)"
              :live="live"
              @changed="changed = true"
            />
          </TabsContent>
          <TabsContent value="browse" class="mt-4">
            <ContentBrowser
              v-if="!browseBlocked"
              :server="id"
              :content-slot="slotKey"
              :sources="slot.sources"
              :reference="slot.mode === 'reference'"
              :fields="slot.fields"
              :field-label="(field: string) => locale.contentFieldLabel(slotKey, field)"
              :field-help="(field: string) => locale.contentFieldHelp(slotKey, field)"
              :can-install="can('servers.content.manage') && !server.suspended_at"
              :live="live"
              @installed="changed = true; tab = 'installed'"
            />
          </TabsContent>
          <TabsContent value="modpacks" class="mt-4">
            <ModpackBrowser
              :server="id"
              :content-slot="slotKey"
              :installed="hasPack"
              :can-install="can('servers.content.manage') && can('servers.update') && !server.suspended_at"
              @installed="changed = true; tab = 'installed'"
            />
          </TabsContent>
        </Tabs>
      </template>
    </div>
  </div>
</template>
