<script setup lang="ts">
// Création d'un serveur : prefab → nom, nœud et ressources → réglages généraux du prefab.
// Page plutôt que fenêtre : le catalogue peut compter des centaines de prefabs. Le prefab choisi
// est dans l'URL (`?prefab=…&version=…`, la version est facultative), ce qui permet d'y
// arriver depuis la fiche d'un prefab et de revenir d'une étape avec le bouton du navigateur.
// Les ressources partent des valeurs par défaut du projet (ADR-0019) quand l'admin peut les lire.
import { toTypedSchema } from '@vee-validate/zod'
import { Boxes, Check } from '@lucide/vue'
import { useForm } from 'vee-validate'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { type RouteLocationRaw, useRoute, useRouter } from 'vue-router'
import { z } from 'zod'

import { usePermissions } from '@/features/account/account.queries'
import { useNodes } from '@/features/nodes/nodes.queries'
import PrefabBlockedBadge from '@/features/prefabs/components/PrefabBlockedBadge.vue'
import PrefabCover from '@/features/prefabs/components/PrefabCover.vue'
import PrefabIcon from '@/features/prefabs/components/PrefabIcon.vue'
import PrefabBrowser from '@/features/prefabs/components/PrefabBrowser.vue'
import ResourceNeedHint from '@/features/prefabs/components/ResourceNeedHint.vue'
import { type CatalogEntry, latestDeployable } from '@/features/prefabs/catalog'
import { need, prefill } from '@/features/prefabs/needs'
import { usePrefab, usePrefabs } from '@/features/prefabs/prefabs.queries'
import CreateServerSettings, { type ServerBasics } from '@/features/servers/components/CreateServerSettings.vue'
import { usePlatformSettings } from '@/features/settings/settings.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { CORES_INPUT, coresToPercent, percentToCores } from '@/shared/lib/format'
import { Button } from '@/shared/ui/button'
import { Card } from '@/shared/ui/card'
import { Empty } from '@/shared/ui/empty'
import { FormControl, FormField, FormItem, FormLabel, FormMessage } from '@/shared/ui/form'
import { Input } from '@/shared/ui/input'
import {
  NumberField,
  NumberFieldContent,
  NumberFieldDecrement,
  NumberFieldIncrement,
  NumberFieldInput,
} from '@/shared/ui/number-field'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Skeleton } from '@/shared/ui/skeleton'

const PATH = '/servers/new'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const read = (key: string) => (typeof route.query[key] === 'string' ? (route.query[key] as string) : undefined)

const { can } = usePermissions()
const { data: platform } = usePlatformSettings(() => can('system.settings.read'))
const { data: prefabs, isPending: catalogPending } = usePrefabs()
const { data: nodes } = useNodes()

/** Catalogue proposé : la version à déployer de chaque prefab. */
const deployable = computed(() => latestDeployable(prefabs.value ?? []))

/** Version choisie : celle de l'URL, sinon la dernière déployable du prefab. */
const pickedEntry = computed(() => {
  const id = read('prefab')
  const version = read('version')
  if (!id) return undefined
  if (version) return prefabs.value?.find((entry) => entry.id === id && entry.version === version && entry.deployable)
  return deployable.value.find((entry) => entry.prefab.id === id)?.prefab
})
const { data: prefab } = usePrefab(
  () => pickedEntry.value?.id,
  () => pickedEntry.value?.version,
)
const basics = ref<ServerBasics>()

/** Étape affichée ; les réglages demandent l'étape 2 validée (perdue au rechargement). */
const step = computed<1 | 2 | 3>(() => (!read('prefab') ? 1 : read('step') === 'settings' && basics.value ? 3 : 2))

/** Nœuds proposés : en ligne, et de test pour un brouillon. */
const onlineNodes = computed(() =>
  (nodes.value ?? []).filter((node) => node.status === 'online' && (!pickedEntry.value?.test_nodes_only || node.trials)),
)

const basicsForm = useForm({
  validationSchema: toTypedSchema(
    z.object({
      name: z.string().trim().min(1, t('prefabs.validation.required')).max(64, t('prefabs.validation.max_length', { max: 64 })),
      node_id: z.string({ required_error: t('prefabs.validation.required') }).uuid(),
      memory_mb: z.number().int().min(64).max(1_048_576),
      cpu_percent: z.number().min(CORES_INPUT.min).max(CORES_INPUT.max),
      disk_mb: z.number().int().min(0).max(104_857_600),
    }),
  ),
  initialValues: defaults(),
})

type BasicResource = 'memory_mb' | 'cpu_percent' | 'disk_mb'

// Le champ `cpu_percent` du formulaire est en cœurs ; l'API, le prefab et les plafonds comptent en
// pourcent (100 = un cœur) : converti au passage à l'étape suivante.
const toField = (resource: BasicResource, value: number) => (resource === 'cpu_percent' ? percentToCores(value) : value)

/**
 * Valeurs par défaut du projet (sinon celles d'origine), relevées à la recommandation du prefab
 * choisi et ramenées au plafond.
 */
function defaults() {
  const entry = pickedEntry.value
  return {
    name: '',
    memory_mb: prefill(entry, 'memory_mb', platform.value?.default_memory_mb ?? 2048, ceiling('memory_mb')),
    cpu_percent: percentToCores(prefill(entry, 'cpu_percent', platform.value?.default_cpu_percent ?? 200, ceiling('cpu_percent'))),
    disk_mb: prefill(entry, 'disk_mb', platform.value?.default_disk_mb ?? 10240, ceiling('disk_mb')),
  }
}

/** Minimum accepté par le champ : celui du prefab, sinon celui de l'API. */
function floor(resource: BasicResource): number {
  return toField(resource, need(pickedEntry.value, resource)?.min ?? (resource === 'disk_mb' ? 0 : 1))
}

/** Plafond du projet (dans l'unité de l'API), appliqué par l'API. */
function ceiling(resource: BasicResource): number | undefined {
  return platform.value?.[`max_${resource}`] ?? undefined
}

/** Plafond tel que le champ s'y arrête. */
function fieldCeiling(resource: BasicResource): number | undefined {
  const max = ceiling(resource)
  return max === undefined ? undefined : toField(resource, max)
}

// Paramètres chargés après l'ouverture : on ne remplace que des champs encore intacts.
watch(platform, () => {
  if (!basicsForm.meta.value.dirty) basicsForm.resetForm({ values: defaults() })
})

// Ressources proposées selon le jeu choisi ; nom et nœud déjà saisis sont gardés.
watch(
  () => pickedEntry.value && `${pickedEntry.value.id}@${pickedEntry.value.version}`,
  (key) => {
    if (!key) return
    const values = defaults()
    for (const resource of ['memory_mb', 'cpu_percent', 'disk_mb'] as const) {
      basicsForm.setFieldValue(resource, values[resource])
    }
  },
  { immediate: true },
)

/**
 * Revient à une étape : par l'historique si l'entrée précédente est cette étape (le catalogue
 * retrouve alors ses filtres), sinon en y naviguant (arrivée directe par un lien).
 */
function goBack(isTarget: (query: Record<string, unknown>) => boolean, fallback: RouteLocationRaw) {
  const back = router.options.history.state.back
  const previous = typeof back === 'string' ? router.resolve(back) : undefined
  if (previous?.path === PATH && isTarget(previous.query)) router.back()
  else void router.push(fallback)
}
const toPicker = () => goBack((query) => !query.prefab, PATH)
const toBasics = () => goBack((query) => !!query.prefab && !query.step, { path: PATH, query: { ...route.query, step: undefined } })

/** Choisir un prefab mène à l'étape 2 ; le catalogue reste dans l'historique avec ses filtres. */
const pickLink = ({ prefab }: CatalogEntry): RouteLocationRaw => ({ path: PATH, query: { prefab: prefab.id, version: prefab.version } })

const next = basicsForm.handleSubmit((values) => {
  basics.value = { ...values, cpu_percent: coresToPercent(values.cpu_percent) } as ServerBasics
  void router.push({ path: PATH, query: { ...route.query, step: 'settings' } })
})

const name = computed(() => {
  const entry = pickedEntry.value
  return entry ? (entry.names[locale.value] ?? entry.names.en ?? entry.id) : ''
})
const description = computed(() => pickedEntry.value?.descriptions[locale.value] ?? pickedEntry.value?.descriptions.en)
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('servers.create.title')">
      <template #meta>{{ t(`servers.create.step${step}`) }}</template>
      <Button variant="ghost" as-child><RouterLink to="/servers/list">{{ t('common.cancel') }}</RouterLink></Button>
    </PageHeader>

    <div class="grid min-w-0 gap-6 p-6">
      <!-- Étapes : celles déjà faites ramènent en arrière. -->
      <ol class="flex items-center gap-2 text-xs text-muted-foreground" :aria-label="t('servers.create.steps')">
        <li v-for="index in 3" :key="index" class="flex items-center gap-2" :aria-current="index === step ? 'step' : undefined">
          <span v-if="index > 1" class="h-px w-4 bg-border sm:w-8" aria-hidden="true" />
          <component
            :is="index < step ? 'button' : 'span'"
            :type="index < step ? 'button' : undefined"
            class="flex items-center gap-2 rounded-full"
            :class="index < step ? 'hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none' : ''"
            @click="index < step && (index === 1 ? toPicker() : toBasics())"
          >
            <span
              class="flex size-6 shrink-0 items-center justify-center rounded-full border"
              :class="index <= step ? 'border-primary bg-primary text-primary-foreground' : ''"
            >
              <Check v-if="index < step" class="size-3" />
              <template v-else>{{ index }}</template>
            </span>
            <span :class="index === step ? 'text-foreground' : 'hidden sm:inline'">{{ t(`servers.create.stepLabel${index}`) }}</span>
          </component>
        </li>
      </ol>

      <!-- 1. Prefab -->
      <PrefabBrowser
        v-if="step === 1"
        :entries="deployable"
        :loading="catalogPending"
        :empty-text="t('servers.create.noPrefab')"
        :link="pickLink"
        view-key="create-server"
      />

      <div v-else-if="catalogPending" class="grid gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
        <Skeleton class="h-96 rounded-xl" /><Skeleton class="hidden h-48 rounded-xl lg:block" />
      </div>

      <Empty v-else-if="!pickedEntry" :icon="Boxes" :title="t('servers.create.notDeployable')">
        {{ t('servers.create.notDeployableHelp') }}
        <template #action>
          <Button variant="outline" size="sm" as-child><RouterLink :to="PATH">{{ t('servers.create.backToCatalog') }}</RouterLink></Button>
        </template>
      </Empty>

      <div v-else class="grid max-w-6xl gap-6 lg:grid-cols-[minmax(0,1fr)_20rem] lg:items-start">
        <!-- Prefab choisi : en tête sur mobile, en colonne à droite sur grand écran. -->
        <aside class="lg:sticky lg:top-6 lg:order-last">
          <PrefabCover :prefab="pickedEntry" class="shadow-card">
            <div class="flex items-center gap-3 p-4 lg:flex-col lg:items-start lg:p-5">
              <PrefabIcon :prefab="pickedEntry" class="size-11 rounded-xl text-lg lg:size-14 lg:text-xl" />
              <div class="grid min-w-0 flex-1 gap-0.5">
                <p class="truncate font-bold">{{ name }}</p>
                <p class="truncate font-mono text-xs text-muted-foreground">{{ `${pickedEntry.id} · ${pickedEntry.version}` }}</p>
              </div>
              <p v-if="description" class="hidden text-[13px] text-muted-foreground lg:block">{{ description }}</p>
              <PrefabBlockedBadge :prefab="pickedEntry" />
              <Button variant="outline" size="sm" class="shrink-0 lg:w-full" @click="toPicker">{{ t('servers.create.change') }}</Button>
            </div>
          </PrefabCover>
        </aside>

        <Card class="p-5 sm:p-6">
          <!-- 2. Nom, nœud, ressources -->
          <form v-if="step === 2" class="grid gap-4 sm:grid-cols-2" @submit.prevent="next">
            <FormField v-slot="{ componentField }" name="name">
              <FormItem class="sm:col-span-2">
                <FormLabel>{{ t('servers.fields.name') }}</FormLabel>
                <FormControl><Input v-bind="componentField" autocomplete="off" /></FormControl>
                <FormMessage />
              </FormItem>
            </FormField>
            <FormField v-slot="{ componentField }" name="node_id">
              <FormItem class="sm:col-span-2">
                <FormLabel>{{ t('servers.fields.node') }}</FormLabel>
                <Select v-bind="componentField">
                  <FormControl>
                    <SelectTrigger class="w-full"><SelectValue :placeholder="t('servers.fields.nodePlaceholder')" /></SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    <SelectItem v-for="node in onlineNodes" :key="node.id" :value="node.id">
                      {{ node.display_name ?? node.name }}
                    </SelectItem>
                  </SelectContent>
                </Select>
                <p v-if="pickedEntry.test_nodes_only" class="text-sm text-muted-foreground">
                  {{ t('servers.create.draft', { reason: pickedEntry.draft_reason || t('servers.create.draftNoReason') }) }}
                </p>
                <p v-if="onlineNodes.length === 0" class="text-sm text-muted-foreground">
                  {{ pickedEntry.test_nodes_only ? t('servers.create.noTestNode') : t('servers.create.noNode') }}
                </p>
                <FormMessage />
              </FormItem>
            </FormField>
            <FormField
              v-for="resource in (['memory_mb', 'cpu_percent', 'disk_mb'] as const)"
              :key="resource"
              v-slot="{ value, handleChange }"
              :name="resource"
            >
              <FormItem>
                <FormLabel>{{ t(`servers.fields.${resource}`) }}</FormLabel>
                <NumberField
                  :model-value="value"
                  :min="floor(resource)"
                  :max="fieldCeiling(resource)"
                  :step="resource === 'cpu_percent' ? CORES_INPUT.step : 512"
                  :format-options="resource === 'cpu_percent' ? CORES_INPUT.formatOptions : undefined"
                  :step-snapping="false"
                  :locale="locale"
                  @update:model-value="handleChange"
                >
                  <NumberFieldContent>
                    <NumberFieldDecrement />
                    <FormControl><NumberFieldInput /></FormControl>
                    <NumberFieldIncrement />
                  </NumberFieldContent>
                </NumberField>
                <ResourceNeedHint :entry="pickedEntry" :resource="resource" />
                <FormMessage />
              </FormItem>
            </FormField>
            <div class="flex justify-between sm:col-span-2">
              <Button type="button" variant="ghost" @click="toPicker">{{ t('common.back') }}</Button>
              <Button type="submit">{{ t('common.continue') }}</Button>
            </div>
          </form>

          <!-- 3. Réglages généraux du prefab -->
          <CreateServerSettings v-else-if="prefab && basics" :prefab="prefab" :basics="basics" @back="toBasics" />
          <Skeleton v-else class="h-64" />
        </Card>
      </div>
    </div>
  </div>
</template>
