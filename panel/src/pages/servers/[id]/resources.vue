<script setup lang="ts">
// Ressources d'un serveur (ADR-0019) : mémoire, CPU et quota disque appliqués à chaud, nombre de
// sauvegardes, espace occupé ; suspension. Seules les valeurs modifiées sont envoyées : ce sont
// elles que l'API confronte aux plafonds du projet.
import { Ban, PlayCircle } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import ResourceNeedHint from '@/features/prefabs/components/ResourceNeedHint.vue'
import { NEEDED_RESOURCES, type NeededResource, need } from '@/features/prefabs/needs'
import { useCatalogEntry } from '@/features/prefabs/prefabs.queries'
import ServerDiskUsage from '@/features/servers/components/ServerDiskUsage.vue'
import { type Server, useServersApi } from '@/features/servers/servers.api'
import { useInvalidateServers, useServer } from '@/features/servers/servers.queries'
import { usePlatformSettings } from '@/features/settings/settings.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { CORES_INPUT, coresToPercent, formatCores, percentToCores } from '@/shared/lib/format'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Button } from '@/shared/ui/button'
import { UnsavedBar } from '@/shared/ui/unsaved-bar'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Label } from '@/shared/ui/label'
import {
  NumberField,
  NumberFieldContent,
  NumberFieldDecrement,
  NumberFieldIncrement,
  NumberFieldInput,
} from '@/shared/ui/number-field'
import { Skeleton } from '@/shared/ui/skeleton'

definePage({ meta: { subnav: 'server' } })

type Resource = 'memory_mb' | 'cpu_percent' | 'disk_mb' | 'backup_limit'
/** Bornes de l'API ; le pas est celui du champ, donc en cœurs pour le CPU. */
const RESOURCES: { key: Resource; min: number; max: number; step: number }[] = [
  { key: 'memory_mb', min: 64, max: 1_048_576, step: 512 },
  { key: 'cpu_percent', min: 1, max: 12_800, step: CORES_INPUT.step },
  { key: 'disk_mb', min: 0, max: 104_857_600, step: 1024 },
  { key: 'backup_limit', min: 0, max: 100, step: 1 },
]

const { t, locale } = useI18n()
const route = useRoute('/servers/[id]/resources')
const id = computed(() => route.params.id)
const { data: server } = useServer(id)
const { can } = usePermissions()
const { data: platform } = usePlatformSettings(() => can('system.settings.read'))
const api = useServersApi()
const { handleError } = useApiError()
const invalidate = useInvalidateServers()

const editable = computed(() => can('servers.update'))
const entry = useCatalogEntry(() => server.value?.prefab)

function isNeeded(key: Resource): key is NeededResource {
  return (NEEDED_RESOURCES as string[]).includes(key)
}

/** Minimum du champ : celui du jeu (ADR-0031), sinon celui de l'API. */
function floor(resource: { key: Resource; min: number }): number {
  return (isNeeded(resource.key) && need(entry.value, resource.key)?.min) || resource.min
}

// Le CPU est saisi en cœurs ; l'API, le prefab et les plafonds le comptent en pourcent (100 = un cœur).
const toField = (key: Resource, value: number) => (key === 'cpu_percent' ? percentToCores(value) : value)
const fromField = (key: Resource, value: number) => (key === 'cpu_percent' ? coresToPercent(value) : value)
/** Valeur d'une ressource telle que les repères l'affichent (plafond…). */
const label = (key: Resource, value: number) => (key === 'cpu_percent' ? formatCores(value, locale.value) : String(value))

/** Valeurs des champs, dans leur unité d'affichage. */
const form = reactive<Record<Resource, number>>({ memory_mb: 0, cpu_percent: 0, disk_mb: 0, backup_limit: 0 })
const errors = ref<FieldErrors>({})
const pending = ref(false)
const confirmSuspend = ref(false)

function load(value: Server) {
  for (const { key } of RESOURCES) form[key] = toField(key, value[key])
}
watch(server, (value) => value && load(value), { immediate: true })

const changed = computed(() => RESOURCES.filter(({ key }) => server.value && fromField(key, form[key]) !== server.value[key]).map(({ key }) => key))

/** Plafond du projet, s'il y en a un et que l'admin peut le lire. */
function ceiling(key: Resource): number | null {
  return platform.value?.[`max_${key}`] ?? null
}

/** Valeur actuelle au-dessus d'un plafond baissé depuis : gardée, signalée. */
function aboveCeiling(key: Resource): boolean {
  const max = ceiling(key)
  return max !== null && !!server.value && (server.value[key] > max || (key === 'disk_mb' && server.value.disk_mb === 0))
}

async function save() {
  if (!server.value || changed.value.length === 0) return
  pending.value = true
  errors.value = {}
  try {
    const body = Object.fromEntries(changed.value.map((key) => [key, fromField(key, form[key])]))
    load(await api.updateResources(server.value.id, body))
    toast.success(t('servers.resources.saved'))
    await invalidate()
  } catch (error) {
    errors.value = handleError(error, RESOURCES.map(({ key }) => key))
  } finally {
    pending.value = false
  }
}

async function setSuspended(suspended: boolean) {
  if (!server.value) return
  pending.value = true
  try {
    await (suspended ? api.suspend(server.value.id) : api.unsuspend(server.value.id))
    toast.success(t(suspended ? 'servers.suspension.suspended' : 'servers.suspension.unsuspended'))
    await invalidate()
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
    confirmSuspend.value = false
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('servers.nav.resources')" />
    <div v-if="!server" class="grid gap-4 p-6"><Skeleton class="h-40" /></div>
    <div v-else class="grid max-w-3xl gap-6 p-6">
      <Card>
        <CardHeader>
          <CardTitle>{{ t('servers.resources.title') }}</CardTitle>
          <CardDescription>{{ t('servers.resources.help') }}</CardDescription>
        </CardHeader>
        <CardContent>
          <form class="grid gap-4 sm:grid-cols-2" @submit.prevent="save">
            <div v-for="resource in RESOURCES" :key="resource.key" class="grid gap-1.5">
              <Label :for="`resource-${resource.key}`">{{ t(`servers.fields.${resource.key}`) }}</Label>
              <NumberField
                :id="`resource-${resource.key}`"
                v-model="form[resource.key]"
                :min="toField(resource.key, floor(resource))"
                :max="toField(resource.key, ceiling(resource.key) ?? resource.max)"
                :step="resource.step"
                :format-options="resource.key === 'cpu_percent' ? CORES_INPUT.formatOptions : undefined"
                :step-snapping="false"
                :locale="locale"
                :disabled="!editable"
              >
                <NumberFieldContent>
                  <NumberFieldDecrement />
                  <NumberFieldInput :aria-invalid="!!errors[resource.key]" />
                  <NumberFieldIncrement />
                </NumberFieldContent>
              </NumberField>
              <p v-if="errors[resource.key]" class="text-sm text-destructive">{{ errors[resource.key] }}</p>
              <p v-else-if="aboveCeiling(resource.key)" class="text-xs text-warning">
                {{ t('servers.resources.aboveCeiling', { max: label(resource.key, ceiling(resource.key)!) }) }}
              </p>
              <p v-else-if="ceiling(resource.key) !== null" class="text-xs text-muted-foreground">
                {{ t('servers.resources.ceiling', { max: label(resource.key, ceiling(resource.key)!) }) }}
              </p>
              <p v-else-if="resource.key === 'disk_mb'" class="text-xs text-muted-foreground">{{ t('servers.resources.diskUnlimited') }}</p>
              <ResourceNeedHint v-if="isNeeded(resource.key)" :entry="entry" :resource="resource.key" />
            </div>
            <UnsavedBar v-if="editable" class="sm:col-span-2" :show="changed.length > 0" :pending="pending" @reset="load(server)" />
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{{ t('servers.disk.title') }}</CardTitle>
          <CardDescription>{{ t('servers.disk.help') }}</CardDescription>
        </CardHeader>
        <CardContent><ServerDiskUsage :server="server" /></CardContent>
      </Card>

      <Card v-if="can('servers.suspend')">
        <CardHeader>
          <CardTitle>{{ t('servers.suspension.title') }}</CardTitle>
          <CardDescription>{{ server.suspended_at ? t('servers.suspension.activeHelp') : t('servers.suspension.help') }}</CardDescription>
        </CardHeader>
        <CardContent>
          <Button v-if="server.suspended_at" variant="secondary" :disabled="pending" @click="setSuspended(false)">
            <PlayCircle />{{ t('servers.suspension.unsuspend') }}
          </Button>
          <Button v-else variant="destructive" :disabled="pending" @click="confirmSuspend = true">
            <Ban />{{ t('servers.suspension.suspend') }}
          </Button>
        </CardContent>
      </Card>
    </div>

    <AlertDialog :open="confirmSuspend" @update:open="(open) => (confirmSuspend = open)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('servers.suspension.confirmTitle', { name: server?.name ?? '' }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('servers.suspension.confirmHelp') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="setSuspended(true)">{{ t('servers.suspension.suspend') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
