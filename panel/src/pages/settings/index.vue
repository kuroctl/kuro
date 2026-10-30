<script setup lang="ts">
// Paramètres du projet (ADR-0019) : valeur par défaut et plafond de chaque ressource des
// serveurs, conservation du journal d'audit, seuils d'alerte des stats (ADR-0043). Seuls les
// champs modifiés sont envoyés.
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import McpSettingsCard from '@/features/settings/components/McpSettingsCard.vue'
import { type PlatformSettings, type SettingsPatch, useSettingsApi } from '@/features/settings/settings.api'
import { useInvalidateSettings, usePlatformSettings } from '@/features/settings/settings.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { CORES_INPUT, coresToPercent, percentToCores } from '@/shared/lib/format'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Label } from '@/shared/ui/label'
import { NumberField, NumberFieldContent, NumberFieldDecrement, NumberFieldIncrement, NumberFieldInput } from '@/shared/ui/number-field'
import { Skeleton } from '@/shared/ui/skeleton'
import { UnsavedBar } from '@/shared/ui/unsaved-bar'

type Resource = 'memory_mb' | 'cpu_percent' | 'disk_mb' | 'backup_limit'
const RESOURCES: Resource[] = ['memory_mb', 'cpu_percent', 'disk_mb', 'backup_limit']
type Alert = 'alert_cpu_percent' | 'alert_memory_percent' | 'alert_minutes'
const ALERTS: Alert[] = ['alert_cpu_percent', 'alert_memory_percent', 'alert_minutes']
type Field = `default_${Resource}` | `max_${Resource}` | 'audit_retention_days' | Alert
const FIELDS: Field[] = [...RESOURCES.flatMap((r) => [`default_${r}`, `max_${r}`] as Field[]), 'audit_retention_days', ...ALERTS]

const { t, locale } = useI18n()
const { can } = usePermissions()
const { data: settings, error } = usePlatformSettings(() => can('system.settings.read'))
const { update } = useSettingsApi()
const { handleError, message } = useApiError()
const invalidate = useInvalidateSettings()
const editable = computed(() => can('system.settings.update'))

/** Allocation CPU saisie en cœurs ; l'API la compte en pourcent (100 = un cœur). Un seuil d'alerte reste en %. */
const isCpu = (field: Field) => field === 'default_cpu_percent' || field === 'max_cpu_percent'

/** Valeurs saisies (CPU en cœurs) : un plafond vide veut dire « aucun ». */
const form = reactive<Record<Field, number | undefined>>(Object.fromEntries(FIELDS.map((f) => [f, undefined])) as Record<Field, number | undefined>)
const errors = ref<FieldErrors>({})
const pending = ref(false)

function load(value: PlatformSettings) {
  for (const field of FIELDS) {
    const stored = value[field]
    form[field] = stored == null ? undefined : isCpu(field) ? percentToCores(stored) : stored
  }
}
watch(settings, (value) => value && load(value), { immediate: true })

/** Champ vidé : `undefined` ou `NaN` selon le NumberField, dans les deux cas « aucun ». */
function parsed(field: Field): number | null {
  const value = form[field]
  if (value == null || Number.isNaN(value)) return null
  return isCpu(field) ? coresToPercent(value) : value
}

const patch = computed<SettingsPatch>(() => {
  const changes: Record<string, number | null> = {}
  for (const field of FIELDS) {
    if (settings.value && parsed(field) !== (settings.value[field] ?? null)) changes[field] = parsed(field)
  }
  return changes as SettingsPatch
})
const dirty = computed(() => Object.keys(patch.value).length > 0)

async function save() {
  pending.value = true
  errors.value = {}
  try {
    load(await update(patch.value))
    toast.success(t('settings.saved'))
    await invalidate()
  } catch (failure) {
    errors.value = handleError(failure, FIELDS)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.settings.project')" />
    <div class="grid max-w-7xl gap-6 p-6">
      <p v-if="!can('system.settings.read')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('settings.forbidden') }}
      </p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <Skeleton v-else-if="!settings" class="h-80" />
      <template v-else>
        <McpSettingsCard :settings="settings" :editable="editable" />
        <form class="grid gap-6" @submit.prevent="save">
          <Card>
            <CardHeader>
              <CardTitle>{{ t('settings.resources.title') }}</CardTitle>
              <CardDescription>{{ t('settings.resources.help') }}</CardDescription>
            </CardHeader>
            <CardContent class="grid gap-4">
              <div class="hidden grid-cols-[1fr_9rem_9rem] gap-3 text-xs font-medium text-muted-foreground sm:grid">
                <span />
                <span>{{ t('settings.resources.default') }}</span>
                <span>{{ t('settings.resources.ceiling') }}</span>
              </div>
              <div v-for="resource in RESOURCES" :key="resource" class="grid items-start gap-3 sm:grid-cols-[1fr_9rem_9rem]">
                <div>
                  <p class="text-sm font-medium">{{ t(`servers.fields.${resource}`) }}</p>
                  <p class="text-xs text-muted-foreground">{{ t(`settings.resources.hint.${resource}`) }}</p>
                </div>
                <div class="grid gap-1">
                  <Label :for="`default_${resource}`" class="sm:sr-only">{{ t('settings.resources.default') }}</Label>
                  <NumberField
                    v-model="form[`default_${resource}`]"
                    :min="resource === 'cpu_percent' ? CORES_INPUT.min : 0"
                    :max="resource === 'cpu_percent' ? CORES_INPUT.max : undefined"
                    :step="resource === 'cpu_percent' ? CORES_INPUT.step : 1"
                    :format-options="resource === 'cpu_percent' ? CORES_INPUT.formatOptions : undefined"
                    :step-snapping="false"
                    :locale="locale"
                    :disabled="!editable"
                  >
                    <NumberFieldContent>
                      <NumberFieldDecrement />
                      <NumberFieldInput :id="`default_${resource}`" :aria-invalid="!!errors[`default_${resource}`]" />
                      <NumberFieldIncrement />
                    </NumberFieldContent>
                  </NumberField>
                  <p v-if="errors[`default_${resource}`]" class="text-xs text-destructive">{{ errors[`default_${resource}`] }}</p>
                </div>
                <div class="grid gap-1">
                  <Label :for="`max_${resource}`" class="sm:sr-only">{{ t('settings.resources.ceiling') }}</Label>
                  <NumberField
                    v-model="form[`max_${resource}`]"
                    :min="resource === 'cpu_percent' ? CORES_INPUT.min : 0"
                    :max="resource === 'cpu_percent' ? CORES_INPUT.max : undefined"
                    :step="resource === 'cpu_percent' ? CORES_INPUT.step : 1"
                    :format-options="resource === 'cpu_percent' ? CORES_INPUT.formatOptions : undefined"
                    :step-snapping="false"
                    :locale="locale"
                    :disabled="!editable"
                  >
                    <NumberFieldContent>
                      <NumberFieldDecrement />
                      <NumberFieldInput :id="`max_${resource}`" :aria-invalid="!!errors[`max_${resource}`]" :placeholder="t('settings.resources.none')" />
                      <NumberFieldIncrement />
                    </NumberFieldContent>
                  </NumberField>
                  <p v-if="errors[`max_${resource}`]" class="text-xs text-destructive">{{ errors[`max_${resource}`] }}</p>
                </div>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>{{ t('settings.audit.title') }}</CardTitle>
              <CardDescription>{{ t('settings.audit.help') }}</CardDescription>
            </CardHeader>
            <CardContent class="grid max-w-xs gap-1.5">
              <Label for="audit_retention_days">{{ t('settings.audit.retention') }}</Label>
              <NumberField
                v-model="form.audit_retention_days"
                :step-snapping="false"
                :locale="locale"
                :disabled="!editable"
              >
                <NumberFieldContent>
                  <NumberFieldDecrement />
                  <NumberFieldInput id="audit_retention_days" :aria-invalid="!!errors.audit_retention_days" />
                  <NumberFieldIncrement />
                </NumberFieldContent>
              </NumberField>
              <p v-if="errors.audit_retention_days" class="text-xs text-destructive">{{ errors.audit_retention_days }}</p>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>{{ t('settings.alerts.title') }}</CardTitle>
              <CardDescription>{{ t('settings.alerts.help') }}</CardDescription>
            </CardHeader>
            <CardContent class="grid gap-4 sm:grid-cols-3">
              <div v-for="field in ALERTS" :key="field" class="grid gap-1.5">
                <Label :for="field">{{ t(`settings.alerts.${field}`) }}</Label>
                <NumberField v-model="form[field]" :min="0" :step-snapping="false" :locale="locale" :disabled="!editable">
                  <NumberFieldContent>
                    <NumberFieldDecrement />
                    <NumberFieldInput :id="field" :aria-invalid="!!errors[field]" />
                    <NumberFieldIncrement />
                  </NumberFieldContent>
                </NumberField>
                <p v-if="errors[field]" class="text-xs text-destructive">{{ errors[field] }}</p>
              </div>
            </CardContent>
          </Card>

          <UnsavedBar v-if="editable" :show="dirty" :pending="pending" @reset="settings && load(settings)" />
        </form>
      </template>
    </div>
  </div>
</template>
