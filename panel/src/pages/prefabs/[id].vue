<script setup lang="ts">
// Détail d'un prefab (ADR-0011) : versions, services et images, réglages, fichiers gérés et
// emplacements de contenu, serveurs déployés, manifest ; suppression d'une version inutilisée.
import { ImageDown, Rocket, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import PrefabBlockedBadge from '@/features/prefabs/components/PrefabBlockedBadge.vue'
import { usePermissions } from '@/features/account/account.queries'
import PrefabCover from '@/features/prefabs/components/PrefabCover.vue'
import PrefabIcon from '@/features/prefabs/components/PrefabIcon.vue'
import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { SECTIONS } from '@/features/prefabs/manifest'
import { usePrefabsApi } from '@/features/prefabs/prefabs.api'
import {
  useInvalidatePrefabAssets,
  useInvalidatePrefabs,
  usePrefab,
  usePrefabs,
  usePrefabServers,
} from '@/features/prefabs/prefabs.queries'
import ServerStateBadge from '@/features/servers/components/ServerStateBadge.vue'
import type { ServerState } from '@/features/servers/state'
import PrefabUpgrades from '@/features/upgrades/components/PrefabUpgrades.vue'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
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
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Skeleton } from '@/shared/ui/skeleton'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/ui/tabs'

const { t, locale: language } = useI18n()
const route = useRoute('/prefabs/[id]')
const router = useRouter()
const id = computed(() => route.params.id)
const { can } = usePermissions()
const { data: catalog } = usePrefabs(() => can('prefabs.read'))
const api = usePrefabsApi()
const invalidate = useInvalidatePrefabs()
const invalidateAssets = useInvalidatePrefabAssets()
const { handleError } = useApiError()

/** Versions de ce prefab, la plus récente d'abord. */
const versions = computed(() =>
  (catalog.value ?? []).filter((prefab) => prefab.id === id.value).sort((a, b) => b.imported_at.localeCompare(a.imported_at)),
)
const version = computed({
  get: () => (typeof route.query.version === 'string' ? route.query.version : versions.value[0]?.version),
  set: (value) => router.replace({ query: { version: value } }),
})
const summary = computed(() => versions.value.find((entry) => entry.version === version.value))
const ref_ = computed(() => (version.value ? { id: id.value, version: version.value } : undefined))
const { data: prefab, isPending } = usePrefab(id, version)
const { data: servers } = usePrefabServers(id, version, () => can('servers.read'))
const locale = usePrefabLocale(ref_)

const name = computed(() => summary.value?.names[language.value] ?? summary.value?.names.en ?? id.value)
const description = computed(() => summary.value?.descriptions[language.value] ?? summary.value?.descriptions.en)
const groups = (section: (typeof SECTIONS)[number]) => Object.entries(prefab.value?.manifest.settings?.[section] ?? {})
const show = (value: unknown) => (value === undefined || value === null || value === '' ? '—' : typeof value === 'object' ? JSON.stringify(value) : String(value))

const deleting = ref(false)
const pending = ref(false)

// Images remises aux dimensions convenues et compressées, côté API.
const optimizing = ref(false)
const kindName = (kind: string) => t(`prefabs.detail.assetKinds.${kind}`)

async function optimize() {
  if (!version.value) return
  optimizing.value = true
  try {
    const report = await api.optimizeAssets(id.value, version.value)
    await invalidateAssets(id.value, version.value)
    const saved = report.optimized.reduce((total, asset) => total + asset.before.bytes - asset.after.bytes, 0)
    if (report.optimized.length) {
      toast.success(t('prefabs.detail.optimized', { count: report.optimized.length, saved: `${Math.round(saved / 1024)} KiB` }, report.optimized.length))
    } else if (!report.failed.length) {
      toast.success(t('prefabs.detail.alreadyOptimized'))
    }
    const small = report.optimized.filter((asset) => asset.small).map((asset) => kindName(asset.kind))
    if (small.length) toast.warning(t('prefabs.detail.tooSmall', { list: small.join(', ') }))
    if (report.failed.length) toast.warning(t('prefabs.detail.optimizeFailed', { list: report.failed.map((entry) => kindName(entry.kind)).join(', ') }))
  } catch (error) {
    handleError(error)
  } finally {
    optimizing.value = false
  }
}

async function remove() {
  if (!version.value) return
  pending.value = true
  try {
    await api.remove(id.value, version.value)
    toast.success(t('prefabs.detail.deleted', { name: name.value, version: version.value }))
    await invalidate()
    deleting.value = false
    await router.push(versions.value.length > 1 ? `/prefabs/${id.value}` : '/prefabs')
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="name">
      <template #meta>
        <span class="font-mono">{{ id }}</span>
        <span v-if="summary">{{ t('prefabs.detail.importedAt', { date: new Date(summary.imported_at).toLocaleDateString(language) }) }}</span>
      </template>
      <Select v-if="versions.length" v-model="version">
        <SelectTrigger class="w-40" :aria-label="t('prefabs.detail.version')">
          <SelectValue>{{ version }}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem v-for="entry in versions" :key="entry.version" :value="entry.version">{{ entry.version }}</SelectItem>
        </SelectContent>
      </Select>
      <Button v-if="can('servers.create') && summary?.deployable" as-child>
        <RouterLink :to="{ path: '/servers/new', query: { prefab: id, version } }"><Rocket />{{ t('servers.create.deployThis') }}</RouterLink>
      </Button>
      <Button
        v-if="can('prefabs.manage') && summary?.assets.length"
        variant="outline"
        :disabled="optimizing"
        :title="t('prefabs.detail.optimizeHelp')"
        @click="optimize"
      >
        <ImageDown :class="{ 'animate-pulse': optimizing }" />{{ t('prefabs.detail.optimize') }}
      </Button>
      <Button v-if="can('prefabs.manage') && summary" variant="outline" :disabled="summary.servers > 0" @click="deleting = true">
        <Trash2 />{{ t('prefabs.detail.delete') }}
      </Button>
    </PageHeader>

    <div v-if="isPending || !summary" class="grid gap-4 p-6"><Skeleton class="h-40" /><Skeleton class="h-64" /></div>
    <div v-else-if="prefab" class="grid gap-6 p-6">
      <PrefabCover :prefab="ref_">
        <div class="flex min-h-32 items-end gap-4 p-5">
          <PrefabIcon :prefab="ref_" class="size-14 rounded-xl text-xl" />
          <div class="grid min-w-0 gap-2">
            <p v-if="description" class="text-sm text-muted-foreground">{{ description }}</p>
            <div class="flex flex-wrap gap-2">
              <Badge variant="secondary">{{ t(`prefabs.catalog.categories.${summary.category}`) }}</Badge>
              <Badge :variant="summary.source === 'official' ? 'info' : 'default'">{{ t(`prefabs.catalog.${summary.source}`) }}</Badge>
              <PrefabBlockedBadge :prefab="summary" />
              <Badge v-for="tag in summary.tags" :key="tag" variant="outline">{{ tag }}</Badge>
            </div>
          </div>
        </div>
      </PrefabCover>

      <Tabs default-value="overview">
        <TabsList>
          <TabsTrigger value="overview">{{ t('prefabs.detail.tabs.overview') }}</TabsTrigger>
          <TabsTrigger value="settings">{{ t('prefabs.detail.tabs.settings') }}</TabsTrigger>
          <TabsTrigger value="files">{{ t('prefabs.detail.tabs.files') }}</TabsTrigger>
          <TabsTrigger v-if="can('servers.read')" value="servers">{{ t('prefabs.detail.tabs.servers') }}</TabsTrigger>
          <TabsTrigger value="manifest">{{ t('prefabs.detail.tabs.manifest') }}</TabsTrigger>
        </TabsList>

        <TabsContent value="overview" class="mt-4 grid gap-4">
          <Card v-for="(service, key) in prefab.manifest.services" :key="key">
            <CardHeader><CardTitle class="font-mono">{{ key }}</CardTitle></CardHeader>
            <CardContent class="grid gap-3 text-sm">
              <p v-if="service.image" class="grid gap-0.5">
                <span class="text-xs text-muted-foreground">{{ t('prefabs.detail.image') }}</span>
                <span class="font-mono break-all">{{ service.image }}</span>
              </p>
              <div v-if="service.ports" class="grid gap-1">
                <span class="text-xs text-muted-foreground">{{ t('prefabs.detail.ports') }}</span>
                <ul class="flex flex-wrap gap-2">
                  <li v-for="(port, portName) in service.ports" :key="portName" class="rounded-md bg-muted px-2 py-1 font-mono text-xs">
                    {{ `${portName} ${port.container}/${port.protocol ?? 'tcp'}` }}
                    <Badge v-if="port.public" variant="success" class="ml-1 h-5 px-2">{{ t('prefabs.detail.publicPort') }}</Badge>
                  </li>
                </ul>
              </div>
              <div v-if="service.volumes" class="grid gap-1">
                <span class="text-xs text-muted-foreground">{{ t('prefabs.detail.volumes') }}</span>
                <ul class="flex flex-wrap gap-2">
                  <li v-for="(volume, volumeName) in service.volumes" :key="volumeName" class="rounded-md bg-muted px-2 py-1 font-mono text-xs">
                    {{ `${volumeName} → ${volume.path}` }}
                  </li>
                </ul>
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="settings" class="mt-4 grid gap-4">
          <template v-for="section in SECTIONS" :key="section">
            <Card v-for="[groupKey, group] in groups(section)" :key="`${section}.${groupKey}`" class="gap-0 pb-0">
              <CardHeader class="pb-4">
                <CardTitle>{{ locale.groupTitle(groupKey) }}</CardTitle>
              </CardHeader>
              <Table class="rounded-t-none">
                <TableHeader>
                  <TableRow>
                    <TableHead>{{ t('prefabs.detail.tabs.settings') }}</TableHead>
                    <TableHead>{{ t('prefabs.detail.fieldType') }}</TableHead>
                    <TableHead>{{ t('prefabs.detail.defaultValue') }}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  <TableRow v-for="(field, fieldKey) in group.fields" :key="fieldKey">
                    <TableCell>
                      <span class="grid">
                        <span class="font-semibold">{{ locale.fieldLabel(String(fieldKey)) }}</span>
                        <span class="font-mono text-xs text-muted-foreground">{{ fieldKey }}</span>
                      </span>
                    </TableCell>
                    <TableCell class="font-mono text-xs">{{ field.type }}</TableCell>
                    <TableCell class="max-w-64 truncate font-mono text-xs">{{ show(field.default) }}</TableCell>
                  </TableRow>
                </TableBody>
              </Table>
            </Card>
          </template>
          <p v-if="SECTIONS.every((section) => groups(section).length === 0)" class="text-sm text-muted-foreground">{{ t('prefabs.detail.noSettings') }}</p>
        </TabsContent>

        <TabsContent value="files" class="mt-4 grid gap-4">
          <Table v-if="prefab.manifest.files && Object.keys(prefab.manifest.files).length">
            <TableBody>
              <TableRow v-for="(file, fileKey) in prefab.manifest.files" :key="fileKey">
                <TableCell>
                  <span class="grid">
                    <span class="font-semibold">{{ fileKey }}</span>
                    <span class="text-xs text-muted-foreground">{{ locale.fileDescription(String(fileKey)) }}</span>
                  </span>
                </TableCell>
                <TableCell class="font-mono text-xs">{{ `${file.service}:${file.path}` }}</TableCell>
                <TableCell><Badge variant="secondary">{{ file.mode }}</Badge></TableCell>
              </TableRow>
            </TableBody>
          </Table>
          <p v-else class="text-sm text-muted-foreground">{{ t('prefabs.detail.noFiles') }}</p>
          <div v-if="prefab.manifest.content" class="grid gap-2">
            <p class="text-[11px] font-bold tracking-[0.07em] text-muted-foreground uppercase">{{ t('prefabs.detail.contentSlots') }}</p>
            <ul class="flex flex-wrap gap-2">
              <li v-for="(slot, slotKey) in prefab.manifest.content" :key="slotKey" class="rounded-md bg-muted px-2.5 py-1.5 text-sm">
                {{ locale.contentTitle(String(slotKey)) }}
                <span class="font-mono text-xs text-muted-foreground">{{ slot.kind }}</span>
              </li>
            </ul>
          </div>
        </TabsContent>

        <TabsContent value="servers" class="mt-4 grid gap-4">
          <PrefabUpgrades v-if="version" :prefab="id" :version="version" />
          <p v-if="!servers?.length" class="text-sm text-muted-foreground">{{ t('prefabs.detail.noServers') }}</p>
          <ul v-else class="grid gap-2">
            <li v-for="server in servers" :key="server.id">
              <RouterLink :to="`/servers/${server.id}`" class="flex items-center justify-between gap-3 rounded-xl border bg-card px-4 py-3 shadow-card hover:bg-muted/50">
                <span class="font-semibold">{{ server.name }}</span>
                <ServerStateBadge :state="server.state as ServerState" />
              </RouterLink>
            </li>
          </ul>
        </TabsContent>

        <TabsContent value="manifest" class="mt-4">
          <!-- `contain: inline-size` : la largeur intrinsèque du bloc ne remonte plus à la grille parente ;
               les longues lignes défilent dans le bloc au lieu d'élargir la page. -->
          <pre class="max-h-[calc(100dvh-12rem)] min-h-48 overflow-auto rounded-xl [contain:inline-size] border bg-console p-4 font-mono text-xs text-console-foreground">{{ JSON.stringify(prefab.manifest, null, 2) }}</pre>
        </TabsContent>
      </Tabs>
    </div>

    <AlertDialog :open="deleting" @update:open="(open) => (deleting = open)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('prefabs.detail.deleteTitle', { name, version }) }}</AlertDialogTitle>
          <AlertDialogDescription>
            {{ t('prefabs.detail.deleteHelp') }}
            <template v-if="summary?.source === 'official'">{{ ` ${t('prefabs.detail.deleteOfficial')}` }}</template>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="remove">{{ t('prefabs.detail.delete') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
