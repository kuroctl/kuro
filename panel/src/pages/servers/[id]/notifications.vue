<script setup lang="ts">
// Notifications d'un serveur (ADR-0044) : ses abonnements, chacun une destination (propre au
// serveur, ou canal partagé des paramètres quand on y a accès) et des événements du serveur ;
// test, modification, suppression et journal des envois de ce serveur.
import { Bell, Pencil, Plus, Send, Trash2 } from '@lucide/vue'
import { computed, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { type Subscription, type SubscriptionRequest, useNotificationsApi } from '@/features/notifications/notifications.api'
import {
  useInvalidateNotifications,
  useNotificationChannels,
  useServerDeliveries,
  useServerSubscriptions,
} from '@/features/notifications/notifications.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Checkbox } from '@/shared/ui/checkbox'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Empty } from '@/shared/ui/empty'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Skeleton } from '@/shared/ui/skeleton'
import { Switch } from '@/shared/ui/switch'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'
import { Textarea } from '@/shared/ui/textarea'

definePage({ meta: { subnav: 'server' } })

const KINDS = ['webhook', 'discord', 'slack', 'email'] as const
const LANGUAGES = ['fr', 'en'] as const
const FIELDS = ['name', 'events', 'channel_id', 'kind', 'url', 'secret', 'recipients', 'language'] as const

const { t, locale } = useI18n()
const route = useRoute('/servers/[id]/notifications')
const id = computed(() => route.params.id)
const { can } = usePermissions()
const readable = computed(() => can('servers.notifications.read'))
const editable = computed(() => can('servers.notifications.manage'))
const api = useNotificationsApi()
const invalidate = useInvalidateNotifications()
const { handleError } = useApiError()
const { data, isPending } = useServerSubscriptions(id, readable)
const { data: deliveries } = useServerDeliveries(id, readable)
// Canaux partagés, seulement quand l'API dit qu'on peut les viser.
const sharedAllowed = computed(() => data.value?.shared_allowed ?? false)
const { data: shared } = useNotificationChannels(sharedAllowed)

const subscriptions = computed(() => data.value?.subscriptions ?? [])
const events = computed(() => data.value?.events ?? [])
const eventLabel = (kind: string) => t(`settings.notifications.events.${kind.replace('.', '_')}`)
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'short', timeStyle: 'medium' }))
const when = (iso: string) => dates.value.format(new Date(iso))
const statusVariant = (status: string) => (status === 'sent' ? 'info' : status === 'pending' ? 'secondary' : 'destructive')
const channelName = (channelId: string) =>
  subscriptions.value.find((s) => s.channel.id === channelId)?.channel.name ?? shared.value?.find((c) => c.id === channelId)?.name ?? '?'

const dialog = ref(false)
const editing = ref<Subscription | null>(null)
const form = reactive({
  name: '',
  events: [] as string[],
  enabled: true,
  destination: 'own' as 'own' | 'shared',
  channel_id: '',
  kind: 'webhook' as string,
  url: '',
  secret: '',
  recipients: '',
  language: 'fr' as string,
})
const errors = ref<FieldErrors>({})
const pending = ref(false)

function open(subscription: Subscription | null) {
  editing.value = subscription
  form.name = subscription?.name ?? ''
  form.events = [...(subscription?.events ?? [])]
  form.enabled = subscription?.enabled ?? true
  form.destination = subscription && !subscription.owned ? 'shared' : 'own'
  form.channel_id = subscription && !subscription.owned ? subscription.channel.id : ''
  form.kind = subscription?.owned ? subscription.channel.kind : 'webhook'
  form.url = ''
  form.secret = ''
  form.recipients = ''
  form.language = subscription?.owned ? subscription.channel.language : locale.value
  errors.value = {}
  dialog.value = true
}

function toggleEvent(event: string, on: boolean) {
  const index = form.events.indexOf(event)
  if (on && index === -1) form.events.push(event)
  if (!on && index !== -1) form.events.splice(index, 1)
}

async function save() {
  pending.value = true
  errors.value = {}
  const body: SubscriptionRequest =
    form.destination === 'shared'
      ? { name: form.name, events: form.events, enabled: form.enabled, channel_id: form.channel_id || null }
      : {
          name: form.name,
          events: form.events,
          enabled: form.enabled,
          kind: form.kind,
          url: form.url || undefined,
          secret: form.secret || undefined,
          recipients:
            form.kind === 'email' && form.recipients.trim()
              ? form.recipients
                  .split(/[\n,;]+/)
                  .map((address) => address.trim())
                  .filter(Boolean)
              : undefined,
          language: form.language,
        }
  try {
    if (editing.value) await api.updateSubscription(id.value, editing.value.id, body)
    else await api.createSubscription(id.value, body)
    dialog.value = false
    await invalidate()
    toast.success(t('servers.notifications.saved'))
  } catch (failure) {
    errors.value = handleError(failure, FIELDS)
  } finally {
    pending.value = false
  }
}

const testing = ref<string | null>(null)
async function test(subscription: Subscription) {
  testing.value = subscription.id
  try {
    const result = await api.testSubscription(id.value, subscription.id)
    if (result.ok) toast.success(t('servers.notifications.testOk', { name: subscription.name }))
    else toast.error(t('servers.notifications.testFailed', { name: subscription.name, error: result.error ?? '?' }))
    await invalidate()
  } catch (failure) {
    handleError(failure)
  } finally {
    testing.value = null
  }
}

async function remove(subscription: Subscription) {
  try {
    await api.removeSubscription(id.value, subscription.id)
    await invalidate()
    toast.success(t('servers.notifications.deleted', { name: subscription.name }))
  } catch (failure) {
    handleError(failure)
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('servers.notifications.title')">
      <Button v-if="editable" size="sm" @click="open(null)"><Plus />{{ t('servers.notifications.add') }}</Button>
    </PageHeader>
    <div class="grid gap-6 p-6">
      <p v-if="!readable" class="text-sm text-muted-foreground">{{ t('servers.notifications.forbidden') }}</p>
      <template v-else>
        <p class="max-w-3xl text-sm text-muted-foreground">{{ t('servers.notifications.help') }}</p>

        <Card class="gap-0 py-0">
          <CardHeader class="px-5 py-4">
            <CardTitle>{{ t('servers.notifications.subscriptions') }}</CardTitle>
            <CardDescription>{{ t('servers.notifications.subscriptionsHelp') }}</CardDescription>
          </CardHeader>
          <CardContent class="px-0 pb-0">
            <div v-if="isPending" class="grid gap-2 p-5"><Skeleton class="h-8" /><Skeleton class="h-8" /></div>
            <Empty v-else-if="!subscriptions.length" :icon="Bell" class="border-0 shadow-none">{{ t('servers.notifications.empty') }}</Empty>
            <ul v-else class="divide-y border-t">
              <li v-for="subscription in subscriptions" :key="subscription.id" class="flex flex-wrap items-center gap-3 px-5 py-3" data-subscription>
                <span class="grid min-w-0 flex-1 gap-1">
                  <span class="flex min-w-0 flex-wrap items-center gap-2">
                    <span class="truncate font-semibold">{{ subscription.name }}</span>
                    <Badge variant="outline">{{ t(`settings.notifications.kinds.${subscription.channel.kind}`) }}</Badge>
                    <Badge v-if="!subscription.owned" variant="secondary">{{ t('servers.notifications.sharedChannel', { name: subscription.channel.name }) }}</Badge>
                    <Badge v-if="!subscription.enabled" variant="secondary">{{ t('settings.notifications.rules.disabled') }}</Badge>
                  </span>
                  <span v-if="subscription.owned" class="truncate font-mono text-xs text-muted-foreground">{{ subscription.channel.url_hint }}</span>
                  <span class="flex flex-wrap gap-1">
                    <Badge v-for="event in subscription.events" :key="event" variant="secondary">{{ eventLabel(event) }}</Badge>
                  </span>
                </span>
                <template v-if="editable">
                  <Button size="sm" variant="outline" :disabled="testing === subscription.id" @click="test(subscription)">
                    <Send />{{ t('settings.notifications.channels.test') }}
                  </Button>
                  <Button size="icon" variant="ghost" :aria-label="t('servers.notifications.edit', { name: subscription.name })" @click="open(subscription)"><Pencil /></Button>
                  <Button size="icon" variant="ghost" :aria-label="t('servers.notifications.delete', { name: subscription.name })" @click="remove(subscription)"><Trash2 /></Button>
                </template>
              </li>
            </ul>
          </CardContent>
        </Card>

        <Card class="gap-0 py-0">
          <CardHeader class="px-5 py-4">
            <CardTitle>{{ t('settings.notifications.deliveries.title') }}</CardTitle>
            <CardDescription>{{ t('servers.notifications.deliveriesHelp') }}</CardDescription>
          </CardHeader>
          <CardContent class="px-0 pb-0">
            <Empty v-if="!deliveries?.length" :icon="Bell" class="border-0 shadow-none">{{ t('settings.notifications.deliveries.empty') }}</Empty>
            <Table v-else>
              <TableHeader>
                <TableRow>
                  <TableHead>{{ t('settings.notifications.deliveries.when') }}</TableHead>
                  <TableHead>{{ t('settings.notifications.deliveries.event') }}</TableHead>
                  <TableHead>{{ t('settings.notifications.deliveries.channel') }}</TableHead>
                  <TableHead>{{ t('settings.notifications.deliveries.status') }}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow v-for="delivery in deliveries" :key="delivery.id" data-delivery>
                  <TableCell class="whitespace-nowrap">{{ when(delivery.created_at) }}</TableCell>
                  <TableCell>{{ eventLabel(String((delivery.event as { kind?: string }).kind ?? '')) }}</TableCell>
                  <TableCell>{{ channelName(delivery.channel_id) }}</TableCell>
                  <TableCell>
                    <Badge :variant="statusVariant(delivery.status)" :title="delivery.last_error ?? undefined">
                      {{ t(`settings.notifications.deliveries.statuses.${delivery.status}`, { attempts: delivery.attempts }) }}
                    </Badge>
                  </TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      </template>
    </div>

    <Dialog :open="dialog" @update:open="(value) => (dialog = value)">
      <DialogContent class="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{{ editing ? t('servers.notifications.editTitle', { name: editing.name }) : t('servers.notifications.add') }}</DialogTitle>
          <DialogDescription>{{ t('servers.notifications.dialogHelp') }}</DialogDescription>
        </DialogHeader>
        <form class="grid gap-4" @submit.prevent="save">
          <div class="grid gap-1.5">
            <Label for="sub-name">{{ t('settings.notifications.fields.name') }}</Label>
            <Input id="sub-name" v-model="form.name" :aria-invalid="!!errors.name" />
            <p v-if="errors.name" class="text-xs text-destructive">{{ errors.name }}</p>
          </div>
          <fieldset class="grid gap-2">
            <legend class="text-sm font-medium">{{ t('settings.notifications.fields.events') }}</legend>
            <label v-for="event in events" :key="event" class="flex items-center gap-2 text-sm">
              <Checkbox :model-value="form.events.includes(event)" @update:model-value="(value) => toggleEvent(event, value === true)" />
              {{ eventLabel(event) }}
            </label>
            <p v-if="errors.events" class="text-xs text-destructive">{{ errors.events }}</p>
          </fieldset>
          <div v-if="sharedAllowed && shared?.length" class="grid gap-1.5">
            <Label for="sub-destination">{{ t('servers.notifications.destination') }}</Label>
            <Select v-model="form.destination">
              <SelectTrigger id="sub-destination" class="w-full"><SelectValue>{{ t(`servers.notifications.destinations.${form.destination}`) }}</SelectValue></SelectTrigger>
              <SelectContent>
                <SelectItem value="own">{{ t('servers.notifications.destinations.own') }}</SelectItem>
                <SelectItem value="shared">{{ t('servers.notifications.destinations.shared') }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div v-if="form.destination === 'shared'" class="grid gap-1.5">
            <Label for="sub-channel">{{ t('settings.notifications.fields.channels') }}</Label>
            <Select v-model="form.channel_id">
              <SelectTrigger id="sub-channel" class="w-full" :aria-invalid="!!errors.channel_id"><SelectValue>{{ channelName(form.channel_id) }}</SelectValue></SelectTrigger>
              <SelectContent>
                <SelectItem v-for="channel in shared ?? []" :key="channel.id" :value="channel.id">{{ channel.name }}</SelectItem>
              </SelectContent>
            </Select>
            <p v-if="errors.channel_id" class="text-xs text-destructive">{{ errors.channel_id }}</p>
          </div>
          <template v-else>
            <div class="grid gap-1.5">
              <Label for="sub-kind">{{ t('settings.notifications.fields.kind') }}</Label>
              <Select v-model="form.kind">
                <SelectTrigger id="sub-kind" class="w-full"><SelectValue>{{ t(`settings.notifications.kinds.${form.kind}`) }}</SelectValue></SelectTrigger>
                <SelectContent>
                  <SelectItem v-for="kind in KINDS" :key="kind" :value="kind">{{ t(`settings.notifications.kinds.${kind}`) }}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div v-if="form.kind === 'email'" class="grid gap-1.5">
              <Label for="sub-recipients">{{ t('settings.notifications.fields.recipients') }}</Label>
              <Textarea id="sub-recipients" v-model="form.recipients" rows="3" class="font-mono" :placeholder="editing?.owned ? t('settings.notifications.fields.unchanged') : 'ops@example.com'" :aria-invalid="!!errors.recipients" />
              <p v-if="errors.recipients" class="text-xs text-destructive">{{ errors.recipients }}</p>
              <p v-else class="text-xs text-muted-foreground">{{ t('settings.notifications.fields.recipientsHelp') }}</p>
            </div>
            <div v-else class="grid gap-1.5">
              <Label for="sub-url">{{ t('settings.notifications.fields.url') }}</Label>
              <Input id="sub-url" v-model="form.url" type="url" class="font-mono" :placeholder="editing?.owned ? t('settings.notifications.fields.unchanged') : 'https://'" :aria-invalid="!!errors.url" />
              <p v-if="errors.url" class="text-xs text-destructive">{{ errors.url }}</p>
              <p v-else class="text-xs text-muted-foreground">{{ t('settings.notifications.fields.urlHelp') }}</p>
            </div>
            <div v-if="form.kind === 'webhook'" class="grid gap-1.5">
              <Label for="sub-secret">{{ t('settings.notifications.fields.secret') }}</Label>
              <Input id="sub-secret" v-model="form.secret" type="password" autocomplete="off" :placeholder="editing?.channel.has_secret ? t('settings.notifications.fields.unchanged') : ''" />
              <p class="text-xs text-muted-foreground">{{ t('settings.notifications.fields.secretHelp') }}</p>
            </div>
            <div class="grid gap-1.5">
              <Label for="sub-language">{{ t('settings.notifications.fields.language') }}</Label>
              <Select v-model="form.language">
                <SelectTrigger id="sub-language" class="w-full"><SelectValue>{{ t(`settings.notifications.languages.${form.language}`) }}</SelectValue></SelectTrigger>
                <SelectContent>
                  <SelectItem v-for="language in LANGUAGES" :key="language" :value="language">{{ t(`settings.notifications.languages.${language}`) }}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </template>
          <div class="flex items-center gap-2">
            <Switch id="sub-enabled" :model-value="form.enabled" @update:model-value="(value) => (form.enabled = value)" />
            <Label for="sub-enabled">{{ t('settings.notifications.fields.enabled') }}</Label>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" @click="dialog = false">{{ t('common.cancel') }}</Button>
            <Button type="submit" :disabled="pending">{{ t('common.save') }}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  </div>
</template>
