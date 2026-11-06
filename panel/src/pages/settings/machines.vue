<script setup lang="ts">
// Réglages › Machines (ADR-0065) : un onglet par fournisseur (identifiants essayés puis chiffrés,
// jamais renvoyés), puis l'adresse de l'API vue par les machines, la plage de ports et, pour
// chaque région, son fournisseur et son emplacement.
import { CircleCheck, Info, MapPinned, Trash2 } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { type MachinesSettings, type ProviderSaved, useMachinesApi } from '@/features/machines/machines.api'
import { useInvalidateMachines, useMachinesSettings, useProviderLocations } from '@/features/machines/machines.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Alert } from '@/shared/ui/alert'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Skeleton } from '@/shared/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/shared/ui/tabs'
import { UnsavedBar } from '@/shared/ui/unsaved-bar'

/** Région sans fournisseur : elle ne loue rien. */
const NONE = '-'
const FIELDS = ['api_url', 'port_range', 'regions', 'token', 'provider'] as const

const { t } = useI18n()
const { can } = usePermissions()
const readable = computed(() => can('system.settings.read'))
const editable = computed(() => can('system.settings.update'))
const { data: settings, error } = useMachinesSettings(readable)
const api = useMachinesApi()
const invalidate = useInvalidateMachines()
const { handleError, message } = useApiError()

const tab = ref('hetzner')
const tokens = reactive<Record<string, string>>({})
const form = reactive({ apiUrl: '', portRange: '', regions: {} as Record<string, { provider: string; location: string }> })
const errors = ref<FieldErrors>({})
const pending = ref<string>()

/** Saisie d'une région (créée par `load`, jamais absente une fois les réglages lus). */
function entry(id: string) {
  return (form.regions[id] ??= { provider: NONE, location: '' })
}
const regions = computed(() => settings.value?.regions ?? [])
const providers = computed(() => settings.value?.providers ?? [])
// Emplacements proposés par chaque fournisseur configuré : choisis dans une liste, jamais tapés.
const { data: locations } = useProviderLocations(() => providers.value.filter((p) => p.configured).map((p) => p.name))
const locationsOf = (provider: string) => locations.value?.[provider] ?? []
const locationLabel = (provider: string, code: string) => {
  const location = locationsOf(provider).find((l) => l.name === code)
  return location ? t('machines.settings.location', { city: location.city, country: location.country, code }) : code
}

function snapshot(value: MachinesSettings) {
  return {
    apiUrl: value.api_url ?? '',
    portRange: value.port_range,
    regions: Object.fromEntries(value.regions.map((r) => [r.region_id, { provider: r.provider ?? NONE, location: r.location ?? '' }])),
  }
}
function load(value: MachinesSettings) {
  Object.assign(form, structuredClone(snapshot(value)))
}
watch(settings, (value) => value && load(value), { immediate: true })

const dirty = computed(() => !!settings.value && JSON.stringify(snapshot(settings.value)) !== JSON.stringify(form))
/** Une région louée demande un emplacement. */
const complete = computed(() => Object.values(form.regions).every((r) => r.provider === NONE || r.location.trim()))

async function run(action: string, task: () => Promise<void>) {
  pending.value = action
  errors.value = {}
  try {
    await task()
  } catch (failure) {
    errors.value = handleError(failure, FIELDS)
  } finally {
    pending.value = undefined
  }
}

const save = () =>
  run('save', async () => {
    load(
      await api.saveSettings({
        api_url: form.apiUrl.trim() || null,
        port_range: form.portRange.trim(),
        regions: Object.entries(form.regions)
          .filter(([, r]) => r.provider !== NONE)
          .map(([region_id, r]) => ({ region_id, provider: r.provider, location: r.location.trim() })),
      }),
    )
    await invalidate()
  })

const saveToken = (provider: string) =>
  run(`token:${provider}`, async () => {
    const saved = await api.setProvider(provider, (tokens[provider] ?? '').trim())
    tokens[provider] = ''
    applySaved(saved, t('machines.settings.tokenSaved', { provider: t(`machines.providers.${provider}`) }))
    await invalidate()
  })

/** Régions créées : réglages relus, et la liste dans le toast (rien de créé : tout existait). */
function applySaved(saved: ProviderSaved, title: string) {
  load(saved.settings)
  const count = saved.created_regions.length
  toast.success(title, {
    description: count
      ? t('machines.settings.regionsCreated', { regions: saved.created_regions.join(', ') }, count)
      : t('machines.settings.regionsUpToDate'),
  })
}

const createRegions = (provider: string) =>
  run(`regions:${provider}`, async () => {
    applySaved(await api.createRegions(provider), t('machines.settings.regionsDone'))
    await invalidate()
  })

const removeToken = (provider: string) =>
  run(`remove:${provider}`, async () => {
    await api.removeProvider(provider)
    await invalidate()
  })
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.settings.machines')" />
    <div class="grid max-w-5xl min-w-0 gap-6 p-6">
      <p v-if="!readable" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('settings.forbidden') }}
      </p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <Skeleton v-else-if="!settings" class="h-80" />
      <template v-else>
        <Card>
          <CardHeader>
            <CardTitle>{{ t('machines.settings.providersTitle') }}</CardTitle>
            <CardDescription>{{ t('machines.settings.providersHelp') }}</CardDescription>
          </CardHeader>
          <CardContent>
            <Tabs v-model="tab">
              <TabsList>
                <TabsTrigger v-for="provider in settings.providers" :key="provider.name" :value="provider.name">
                  {{ t(`machines.providers.${provider.name}`) }}
                </TabsTrigger>
              </TabsList>
              <TabsContent v-for="provider in settings.providers" :key="provider.name" :value="provider.name" class="grid gap-4 pt-4">
                <p class="flex items-center gap-2 text-sm">
                  <Badge v-if="provider.configured" variant="success"><CircleCheck />{{ t('machines.settings.configured') }}</Badge>
                  <Badge v-else variant="secondary">{{ t('machines.settings.notConfigured') }}</Badge>
                </p>
                <form v-if="editable" class="grid gap-1.5 sm:max-w-md" @submit.prevent="saveToken(provider.name)">
                  <Label :for="`token-${provider.name}`">{{ t('machines.settings.token') }}</Label>
                  <Input
                    :id="`token-${provider.name}`"
                    v-model="tokens[provider.name]"
                    type="password"
                    autocomplete="new-password"
                    :placeholder="provider.configured ? t('machines.settings.tokenStored') : ''"
                    :aria-invalid="!!errors.token || !!errors.provider"
                  />
                  <p v-if="errors.token || errors.provider" class="text-xs text-destructive">{{ errors.token ?? errors.provider }}</p>
                  <p v-else class="text-xs text-muted-foreground">{{ t(`machines.settings.tokenHelp.${provider.name}`) }}</p>
                  <div class="flex flex-wrap gap-2 pt-2">
                    <Button type="submit" :disabled="!!pending || !tokens[provider.name]?.trim()">{{ t('machines.settings.saveToken') }}</Button>
                    <Button
                      v-if="provider.configured"
                      type="button"
                      variant="outline"
                      :disabled="!!pending"
                      @click="createRegions(provider.name)"
                    >
                      <MapPinned />{{ t('machines.settings.createRegions') }}
                    </Button>
                    <Button
                      v-if="provider.configured"
                      type="button"
                      variant="ghost"
                      class="text-destructive"
                      :disabled="!!pending"
                      @click="removeToken(provider.name)"
                    >
                      <Trash2 />{{ t('machines.settings.removeToken') }}
                    </Button>
                  </div>
                </form>
              </TabsContent>
            </Tabs>
          </CardContent>
        </Card>

        <form class="grid gap-6" @submit.prevent="save">
          <Card>
            <CardHeader>
              <CardTitle>{{ t('machines.settings.generalTitle') }}</CardTitle>
              <CardDescription>{{ t('machines.settings.generalHelp') }}</CardDescription>
            </CardHeader>
            <CardContent class="grid gap-4 sm:grid-cols-2">
              <div class="grid content-start gap-1.5">
                <Label for="machines-api-url">{{ t('machines.settings.apiUrl') }}</Label>
                <Input
                  id="machines-api-url"
                  v-model="form.apiUrl"
                  placeholder="https://panel.example.com:8443"
                  autocomplete="off"
                  spellcheck="false"
                  :disabled="!editable"
                  :aria-invalid="!!errors.api_url"
                />
                <p v-if="errors.api_url" class="text-xs text-destructive">{{ errors.api_url }}</p>
                <p v-else class="text-xs text-muted-foreground">{{ t('machines.settings.apiUrlHelp') }}</p>
              </div>
              <div class="grid content-start gap-1.5">
                <Label for="machines-ports">{{ t('machines.settings.portRange') }}</Label>
                <Input
                  id="machines-ports"
                  v-model="form.portRange"
                  autocomplete="off"
                  spellcheck="false"
                  :disabled="!editable"
                  :aria-invalid="!!errors.port_range"
                />
                <p v-if="errors.port_range" class="text-xs text-destructive">{{ errors.port_range }}</p>
                <p v-else class="text-xs text-muted-foreground">{{ t('machines.settings.portRangeHelp') }}</p>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>{{ t('machines.settings.regionsTitle') }}</CardTitle>
              <CardDescription>{{ t('machines.settings.regionsHelp') }}</CardDescription>
            </CardHeader>
            <CardContent class="grid gap-3">
              <Alert v-if="regions.length === 0" variant="info"><Info />{{ t('machines.settings.noRegion') }}</Alert>
              <p v-if="errors.regions" class="text-xs text-destructive">{{ errors.regions }}</p>
              <div
                v-for="region in regions"
                :key="region.region_id"
                class="grid items-center gap-3 sm:grid-cols-[minmax(0,1fr)_12rem_16rem]"
                :data-region="region.slug"
              >
                <p class="truncate font-semibold">{{ region.name }}</p>
                <Select v-model="entry(region.region_id).provider" :disabled="!editable">
                  <SelectTrigger class="w-full" :aria-label="t('machines.settings.regionProvider', { region: region.name })">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem :value="NONE">{{ t('machines.settings.noProvider') }}</SelectItem>
                    <SelectItem v-for="provider in providers" :key="provider.name" :value="provider.name">
                      {{ t(`machines.providers.${provider.name}`) }}
                    </SelectItem>
                  </SelectContent>
                </Select>
                <Select
                  v-model="entry(region.region_id).location"
                  :disabled="!editable || entry(region.region_id).provider === NONE"
                >
                  <SelectTrigger class="w-full" :aria-label="t('machines.settings.regionLocation', { region: region.name })">
                    <SelectValue :placeholder="t('machines.settings.locationPlaceholder')">
                      {{ entry(region.region_id).location ? locationLabel(entry(region.region_id).provider, entry(region.region_id).location) : undefined }}
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem v-for="location in locationsOf(entry(region.region_id).provider)" :key="location.name" :value="location.name">
                      {{ locationLabel(entry(region.region_id).provider, location.name) }}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </CardContent>
          </Card>

          <UnsavedBar v-if="editable" :show="dirty" :pending="pending === 'save'" :can-save="complete" @reset="settings && load(settings)" />
        </form>
      </template>
    </div>
  </div>
</template>
