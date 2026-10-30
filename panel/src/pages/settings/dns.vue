<script setup lang="ts">
// DNS des serveurs et des nœuds (ADR-0026) : zone chez le fournisseur, jeton d'API (jamais
// renvoyé : un champ vide garde celui enregistré), test, synchronisation immédiate, suppression
// de la zone, et liste des enregistrements créés par Kuro.
import { CircleAlert, Info, RefreshCw, Trash2, Wifi } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import DnsRecordsTable from '@/features/dns/components/DnsRecordsTable.vue'
import { type DnsSettings, type DnsSettingsRequest, useDnsApi } from '@/features/dns/dns.api'
import { useDnsSettings, useInvalidateDns } from '@/features/dns/dns.queries'
import { type FieldErrors, problemCode, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Alert } from '@/shared/ui/alert'
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
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { NumberField, NumberFieldContent, NumberFieldDecrement, NumberFieldIncrement, NumberFieldInput } from '@/shared/ui/number-field'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Skeleton } from '@/shared/ui/skeleton'
import { Switch } from '@/shared/ui/switch'

const PROVIDERS = ['cloudflare'] as const
const DEFAULT_NODES_LABEL = 'nodes'
const DEFAULT_TTL = 300
/** Champs que l'API peut refuser ; `label` désigne le sous-domaine des nœuds. */
const FIELDS = ['base_domain', 'label', 'token', 'ttl'] as const
/** Refus du fournisseur : affichés sur place, avec leur explication. */
const PROVIDER_ERRORS = ['dns_provider_unauthorized', 'dns_zone_not_found', 'dns_provider_unavailable', 'unknown_dns_provider']

const { t, locale } = useI18n()
const { can } = usePermissions()
const allowed = computed(() => can('system.dns.manage'))
const { data: settings, error } = useDnsSettings(allowed)
const api = useDnsApi()
const invalidate = useInvalidateDns()
const { handleError, message } = useApiError()

const form = reactive({
  provider: 'cloudflare' as string,
  baseDomain: '',
  nodesLabel: DEFAULT_NODES_LABEL,
  token: '',
  ttl: DEFAULT_TTL as number | undefined,
  enabled: true,
})
const errors = ref<FieldErrors>({})
const providerError = ref<string>()
/** Action en cours : un seul bouton actif à la fois. */
const pending = ref<'save' | 'test' | 'sync' | 'remove'>()
const confirmRemove = ref(false)

function load(value: DnsSettings) {
  form.provider = value.provider ?? 'cloudflare'
  form.baseDomain = value.base_domain ?? ''
  form.nodesLabel = value.nodes_label ?? DEFAULT_NODES_LABEL
  form.ttl = value.ttl ?? DEFAULT_TTL
  form.enabled = value.configured ? value.enabled : true
  form.token = ''
}
watch(settings, (value) => value && load(value), { immediate: true })

const configured = computed(() => settings.value?.configured ?? false)

/** Erreur d'une action : refus du fournisseur sur place, erreurs de champ sous chaque champ. */
function fail(failure: unknown) {
  if (PROVIDER_ERRORS.includes(problemCode(failure) ?? '')) {
    providerError.value = message(failure)
    return
  }
  errors.value = handleError(failure, FIELDS)
}

async function run(action: NonNullable<typeof pending.value>, task: () => Promise<void>) {
  pending.value = action
  errors.value = {}
  providerError.value = undefined
  try {
    await task()
  } catch (failure) {
    fail(failure)
  } finally {
    pending.value = undefined
  }
}

const save = () =>
  run('save', async () => {
    const body: DnsSettingsRequest = {
      provider: form.provider,
      base_domain: form.baseDomain.trim(),
      nodes_label: form.nodesLabel.trim() || DEFAULT_NODES_LABEL,
      ttl: form.ttl == null || Number.isNaN(form.ttl) ? null : form.ttl,
      enabled: form.enabled,
    }
    // Le jeton ne part que s'il a été saisi : absent, l'API garde celui enregistré.
    if (form.token.trim()) body.token = form.token.trim()
    load(await api.configure(body))
    toast.success(t('dns.saved'))
    await invalidate()
  })

const test = () =>
  run('test', async () => {
    const result = await api.test()
    toast.success(t('dns.tested', { count: result.records }))
  })

const sync = () =>
  run('sync', async () => {
    const result = await api.sync()
    toast.success(t('dns.synced', { ...result }))
    await invalidate()
  })

const remove = () =>
  run('remove', async () => {
    await api.remove()
    confirmRemove.value = false
    toast.success(t('dns.removed'))
    await invalidate()
  })
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.settings.dns')" />
    <div class="grid max-w-7xl min-w-0 gap-6 p-6">
      <p v-if="!allowed" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('dns.forbidden') }}
      </p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <Skeleton v-else-if="!settings" class="h-80" />
      <template v-else>
        <form @submit.prevent="save">
          <Card>
            <CardHeader>
              <CardTitle>{{ t('dns.zone.title') }}</CardTitle>
              <CardDescription>{{ t('dns.zone.help') }}</CardDescription>
            </CardHeader>
            <CardContent class="grid gap-4">
              <Alert variant="info"><Info />{{ t('dns.zone.tokenScope') }}</Alert>
              <Alert v-if="providerError" variant="destructive" role="alert"><CircleAlert />{{ providerError }}</Alert>

              <div class="grid gap-4 sm:grid-cols-2">
                <div class="grid content-start gap-1.5">
                  <Label for="dns-provider">{{ t('dns.fields.provider') }}</Label>
                  <Select v-model="form.provider">
                    <SelectTrigger id="dns-provider" class="w-full">
                      <SelectValue>{{ t(`dns.providers.${form.provider}`) }}</SelectValue>
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem v-for="provider in PROVIDERS" :key="provider" :value="provider">{{ t(`dns.providers.${provider}`) }}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div class="grid content-start gap-1.5">
                  <Label for="dns-base-domain">{{ t('dns.fields.baseDomain') }}</Label>
                  <Input
                    id="dns-base-domain"
                    v-model="form.baseDomain"
                    placeholder="play.example.com"
                    autocomplete="off"
                    spellcheck="false"
                    :aria-invalid="!!errors.base_domain"
                  />
                  <p v-if="errors.base_domain" class="text-xs text-destructive">{{ errors.base_domain }}</p>
                  <p v-else class="text-xs text-muted-foreground">{{ t('dns.fields.baseDomainHelp', { domain: form.baseDomain || 'play.example.com' }) }}</p>
                </div>
                <div class="grid content-start gap-1.5">
                  <Label for="dns-nodes-label">{{ t('dns.fields.nodesLabel') }}</Label>
                  <Input
                    id="dns-nodes-label"
                    v-model="form.nodesLabel"
                    :placeholder="DEFAULT_NODES_LABEL"
                    autocomplete="off"
                    spellcheck="false"
                    :aria-invalid="!!errors.label"
                  />
                  <p v-if="errors.label" class="text-xs text-destructive">{{ errors.label }}</p>
                  <p v-else class="text-xs text-muted-foreground">
                    {{ t('dns.fields.nodesLabelHelp', { example: `node-1.${form.nodesLabel || DEFAULT_NODES_LABEL}.${form.baseDomain || 'play.example.com'}` }) }}
                  </p>
                </div>
                <div class="grid content-start gap-1.5">
                  <Label for="dns-token">{{ t('dns.fields.token') }}</Label>
                  <Input
                    id="dns-token"
                    v-model="form.token"
                    type="password"
                    autocomplete="new-password"
                    :placeholder="settings.has_token ? t('dns.fields.tokenStored') : ''"
                    :aria-invalid="!!errors.token"
                  />
                  <p v-if="errors.token" class="text-xs text-destructive">{{ errors.token }}</p>
                  <p v-else class="text-xs text-muted-foreground">{{ t(settings.has_token ? 'dns.fields.tokenKeep' : 'dns.fields.tokenHelp') }}</p>
                </div>
                <div class="grid content-start gap-1.5">
                  <Label for="dns-ttl">{{ t('dns.fields.ttl') }}</Label>
                  <NumberField v-model="form.ttl" :min="60" :max="86400" :step-snapping="false" :locale="locale">
                    <NumberFieldContent>
                      <NumberFieldDecrement />
                      <NumberFieldInput id="dns-ttl" :aria-invalid="!!errors.ttl" />
                      <NumberFieldIncrement />
                    </NumberFieldContent>
                  </NumberField>
                  <p v-if="errors.ttl" class="text-xs text-destructive">{{ errors.ttl }}</p>
                  <p v-else class="text-xs text-muted-foreground">{{ t('dns.fields.ttlHelp') }}</p>
                </div>
                <div class="flex items-center gap-3 self-center">
                  <Switch id="dns-enabled" v-model="form.enabled" />
                  <Label for="dns-enabled">{{ t('dns.fields.enabled') }}</Label>
                </div>
              </div>

              <div class="flex flex-wrap items-center gap-2 border-t pt-4">
                <Button type="submit" :disabled="!!pending || !form.baseDomain.trim()">{{ t('common.save') }}</Button>
                <template v-if="configured">
                  <Button type="button" variant="outline" :disabled="!!pending" @click="test"><Wifi />{{ t('dns.actions.test') }}</Button>
                  <Button type="button" variant="outline" :disabled="!!pending" @click="sync"><RefreshCw />{{ t('dns.actions.sync') }}</Button>
                  <Button type="button" variant="ghost" class="ml-auto text-destructive" :disabled="!!pending" @click="confirmRemove = true">
                    <Trash2 />{{ t('dns.actions.remove') }}
                  </Button>
                </template>
              </div>
            </CardContent>
          </Card>
        </form>

        <Card>
          <CardHeader>
            <CardTitle>{{ t('dns.records.title') }}</CardTitle>
            <CardDescription>{{ t('dns.records.help') }}</CardDescription>
          </CardHeader>
          <CardContent>
            <DnsRecordsTable :records="settings.records" />
          </CardContent>
        </Card>
      </template>
    </div>

    <AlertDialog v-model:open="confirmRemove">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('dns.remove.title') }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('dns.remove.help') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="!!pending" @click="remove">{{ t('dns.remove.confirm') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
