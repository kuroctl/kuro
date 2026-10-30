<script setup lang="ts">
// Assistant de création : prefab → nom, nœud et ressources → réglages généraux du prefab.
// Les ressources partent des valeurs par défaut du projet (ADR-0019) quand l'admin peut les lire.
import { toTypedSchema } from '@vee-validate/zod'
import { Check, Plus } from '@lucide/vue'
import { useForm } from 'vee-validate'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { z } from 'zod'

import { usePermissions } from '@/features/account/account.queries'
import { useNodes } from '@/features/nodes/nodes.queries'
import PrefabIcon from '@/features/prefabs/components/PrefabIcon.vue'
import PrefabBlockedBadge from '@/features/prefabs/components/PrefabBlockedBadge.vue'
import ResourceNeedHint from '@/features/prefabs/components/ResourceNeedHint.vue'
import { need, prefill } from '@/features/prefabs/needs'
import { usePrefab, usePrefabs } from '@/features/prefabs/prefabs.queries'
import { usePlatformSettings } from '@/features/settings/settings.queries'
import { Button } from '@/shared/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/shared/ui/dialog'
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

import CreateServerSettings, { type ServerBasics } from './CreateServerSettings.vue'

const { t, locale } = useI18n()
const open = ref(false)
const step = ref<1 | 2 | 3>(1)
const picked = ref<{ id: string; version: string }>()
const basics = ref<ServerBasics>()

const { can } = usePermissions()
const { data: platform } = usePlatformSettings(() => can('system.settings.read'))
const { data: prefabs } = usePrefabs()
const { data: nodes } = useNodes()
const { data: prefab } = usePrefab(
  () => picked.value?.id,
  () => picked.value?.version,
)

/**
 * Dernière version déployable de chaque prefab : une version publiée de préférence, sinon un
 * brouillon (`x-status`), qui ne se déploie que sur un nœud de test (ADR-0011).
 */
const catalog = computed(() => {
  const latest = new Map<string, NonNullable<typeof prefabs.value>[number]>()
  for (const entry of prefabs.value ?? []) {
    if (!entry.deployable) continue
    const current = latest.get(entry.id)
    const better =
      !current ||
      (current.test_nodes_only && !entry.test_nodes_only) ||
      (current.test_nodes_only === entry.test_nodes_only && compareVersions(entry.version, current.version) > 0)
    if (better) latest.set(entry.id, entry)
  }
  return [...latest.values()]
})

function compareVersions(a: string, b: string): number {
  const parts = (v: string) => v.split(/[.-]/).map((p) => Number.parseInt(p, 10) || 0)
  const [x, y] = [parts(a), parts(b)]
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) - (y[i] ?? 0)
  }
  return 0
}

function localized(texts: Record<string, string>, fallback: string): string {
  return texts[locale.value] ?? texts.en ?? fallback
}

/** Version choisie, telle que le catalogue la décrit (besoins en ressources, ADR-0031). */
const pickedEntry = computed(() =>
  (prefabs.value ?? []).find((entry) => entry.id === picked.value?.id && entry.version === picked.value?.version),
)
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
      cpu_percent: z.number().int().min(1).max(12_800),
      disk_mb: z.number().int().min(0).max(104_857_600),
    }),
  ),
  initialValues: defaults(),
})

/**
 * Valeurs par défaut du projet (sinon celles d'origine), relevées à la recommandation du prefab
 * choisi et ramenées au plafond.
 */
function defaults() {
  const entry = pickedEntry.value
  return {
    name: '',
    memory_mb: prefill(entry, 'memory_mb', platform.value?.default_memory_mb ?? 2048, ceiling('memory_mb')),
    cpu_percent: prefill(entry, 'cpu_percent', platform.value?.default_cpu_percent ?? 200, ceiling('cpu_percent')),
    disk_mb: prefill(entry, 'disk_mb', platform.value?.default_disk_mb ?? 10240, ceiling('disk_mb')),
  }
}

/** Minimum accepté par le champ : celui du prefab, sinon celui de l'API. */
function floor(resource: 'memory_mb' | 'cpu_percent' | 'disk_mb'): number {
  return need(pickedEntry.value, resource)?.min ?? (resource === 'disk_mb' ? 0 : 1)
}

/** Plafond du projet, appliqué par l'API ; le champ s'y arrête. */
function ceiling(resource: 'memory_mb' | 'cpu_percent' | 'disk_mb'): number | undefined {
  return platform.value?.[`max_${resource}`] ?? undefined
}

// Paramètres chargés après l'ouverture : on ne remplace que des champs encore intacts.
watch(platform, () => {
  if (!basicsForm.meta.value.dirty) basicsForm.resetForm({ values: defaults() })
})

function pick(id: string, version: string) {
  picked.value = { id, version }
  // Ressources proposées selon le jeu choisi ; nom et nœud déjà saisis sont gardés.
  const values = defaults()
  for (const resource of ['memory_mb', 'cpu_percent', 'disk_mb'] as const) {
    basicsForm.setFieldValue(resource, values[resource])
  }
  step.value = 2
}

const next = basicsForm.handleSubmit((values) => {
  basics.value = values as ServerBasics
  step.value = 3
})

function reset(isOpen: boolean) {
  open.value = isOpen
  if (!isOpen) {
    step.value = 1
    picked.value = undefined
    basics.value = undefined
    basicsForm.resetForm({ values: defaults() })
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="reset">
    <DialogTrigger as-child>
      <Button size="sm"><Plus />{{ t('servers.create.action') }}</Button>
    </DialogTrigger>
    <DialogContent class="max-h-[90dvh] overflow-y-auto sm:max-w-2xl">
      <DialogHeader>
        <DialogTitle>{{ t('servers.create.title') }}</DialogTitle>
        <DialogDescription>{{ t(`servers.create.step${step}`) }}</DialogDescription>
      </DialogHeader>

      <ol class="flex items-center gap-2 text-xs text-muted-foreground">
        <li v-for="index in 3" :key="index" class="flex items-center gap-2">
          <span
            class="flex size-6 items-center justify-center rounded-full border"
            :class="index <= step ? 'border-primary bg-primary text-primary-foreground' : ''"
          >
            <Check v-if="index < step" class="size-3" />
            <template v-else>{{ index }}</template>
          </span>
          <span :class="index === step ? 'text-foreground' : ''">{{ t(`servers.create.stepLabel${index}`) }}</span>
        </li>
      </ol>

      <!-- 1. Prefab -->
      <div v-if="step === 1" class="grid gap-3 sm:grid-cols-2">
        <button
          v-for="entry in catalog"
          :key="entry.id"
          type="button"
          class="rounded-lg border p-4 text-left transition-colors hover:border-primary hover:bg-accent"
          @click="pick(entry.id, entry.version)"
        >
          <span class="flex items-center gap-3">
            <PrefabIcon :prefab="entry" class="size-10 text-base" />
            <span class="font-medium">{{ localized(entry.names, entry.id) }}</span>
          </span>
          <p class="mt-1 line-clamp-2 text-sm text-muted-foreground">{{ localized(entry.descriptions, '') }}</p>
          <p class="mt-2 flex flex-wrap items-center gap-2 font-mono text-xs text-muted-foreground">
            {{ `${entry.id} · ${entry.version}` }}
            <PrefabBlockedBadge :prefab="entry" />
          </p>
        </button>
        <p v-if="catalog.length === 0" class="text-sm text-muted-foreground">{{ t('servers.create.noPrefab') }}</p>
      </div>

      <!-- 2. Nom, nœud, ressources -->
      <form v-else-if="step === 2" class="grid gap-4 sm:grid-cols-2" @submit.prevent="next">
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
            <p v-if="pickedEntry?.test_nodes_only" class="text-sm text-muted-foreground">
              {{ t('servers.create.draft', { reason: pickedEntry.draft_reason || t('servers.create.draftNoReason') }) }}
            </p>
            <p v-if="onlineNodes.length === 0" class="text-sm text-muted-foreground">
              {{ pickedEntry?.test_nodes_only ? t('servers.create.noTestNode') : t('servers.create.noNode') }}
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
            <NumberField :model-value="value" :min="floor(resource)" :max="ceiling(resource)" :step-snapping="false" :locale="locale" :step="resource === 'cpu_percent' ? 50 : 512" @update:model-value="handleChange">
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
          <Button type="button" variant="ghost" @click="step = 1">{{ t('common.back') }}</Button>
          <Button type="submit">{{ t('common.continue') }}</Button>
        </div>
      </form>

      <!-- 3. Réglages généraux du prefab -->
      <CreateServerSettings
        v-else-if="prefab && basics"
        :prefab="prefab"
        :basics="basics"
        @back="step = 2"
        @created="reset(false)"
      />
    </DialogContent>
  </Dialog>
</template>
