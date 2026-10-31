<script setup lang="ts">
// Notifications et webhooks (ADR-0044) : canaux (webhook signé, Discord, Slack ; URL et secret
// jamais relus), règles (portée, événements, canaux) et journal des envois.
import { Bell, Pencil, Plus, Send, Trash2 } from '@lucide/vue'
import { computed, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { useNodes } from '@/features/nodes/nodes.queries'
import { type Channel, type ChannelRequest, type Rule, type RuleRequest, useNotificationsApi } from '@/features/notifications/notifications.api'
import {
  useInvalidateNotifications,
  useNotificationChannels,
  useNotificationDeliveries,
  useNotificationEvents,
  useNotificationRules,
} from '@/features/notifications/notifications.queries'
import { useServers } from '@/features/servers/servers.queries'
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

const KINDS = ['webhook', 'discord', 'slack', 'email'] as const
const LANGUAGES = ['fr', 'en'] as const
const SCOPES = ['all', 'node', 'server'] as const
const CHANNEL_FIELDS = ['name', 'kind', 'url', 'secret', 'recipients', 'language'] as const
const RULE_FIELDS = ['name', 'scope', 'node_id', 'server_id', 'events', 'channels'] as const

const { t, locale } = useI18n()
const { can } = usePermissions()
const readable = computed(() => can('system.notifications.read'))
const editable = computed(() => can('system.notifications.manage'))
const api = useNotificationsApi()
const invalidate = useInvalidateNotifications()
const { handleError } = useApiError()
const { data: channels, isPending: channelsPending } = useNotificationChannels(readable)
const { data: rules } = useNotificationRules(readable)
const { data: events } = useNotificationEvents(readable)
const { data: deliveries } = useNotificationDeliveries(readable)
const { data: nodes } = useNodes()
const { data: servers } = useServers()

const channelName = (id: string) => channels.value?.find((channel) => channel.id === id)?.name ?? '?'
const nodeName = (id: string | null | undefined) => nodes.value?.find((node) => node.id === id)?.name ?? '?'
const serverName = (id: string | null | undefined) => servers.value?.find((server) => server.id === id)?.name ?? '?'
const eventLabel = (kind: string) => t(`settings.notifications.events.${kind.replace('.', '_')}`)
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'short', timeStyle: 'medium' }))
const when = (iso: string) => dates.value.format(new Date(iso))

/** Cible d'un envoi : le serveur ou le nœud de l'événement. */
function target(delivery: { event: Record<string, unknown> }): string {
  const event = delivery.event as { server?: { name?: string }; node?: { name?: string } }
  return event.server?.name ?? event.node?.name ?? ''
}

// --- Canaux
const channelDialog = ref(false)
const editingChannel = ref<Channel | null>(null)
const channelForm = reactive({ name: '', kind: 'webhook' as string, url: '', secret: '', recipients: '', language: 'fr' as string, enabled: true })
const channelErrors = ref<FieldErrors>({})
const pending = ref(false)

/** Une adresse par ligne, virgule ou point-virgule acceptés. */
function splitRecipients(text: string): string[] {
  return text
    .split(/[\n,;]+/)
    .map((address) => address.trim())
    .filter(Boolean)
}

function openChannel(channel: Channel | null) {
  editingChannel.value = channel
  channelForm.name = channel?.name ?? ''
  channelForm.kind = channel?.kind ?? 'webhook'
  channelForm.url = ''
  channelForm.secret = ''
  channelForm.recipients = ''
  channelForm.language = channel?.language ?? locale.value
  channelForm.enabled = channel?.enabled ?? true
  channelErrors.value = {}
  channelDialog.value = true
}

async function saveChannel() {
  pending.value = true
  channelErrors.value = {}
  const body: ChannelRequest = {
    name: channelForm.name,
    kind: channelForm.kind,
    language: channelForm.language,
    enabled: channelForm.enabled,
    url: channelForm.url || undefined,
    // En modification, un secret vide est envoyé seulement pour le retirer ; à la création il
    // est simplement absent.
    secret: channelForm.secret || (editingChannel.value ? undefined : undefined),
    // Canal e-mail : une adresse par ligne ; vide en modification, celles en place sont gardées.
    recipients: channelForm.kind === 'email' && channelForm.recipients.trim() ? splitRecipients(channelForm.recipients) : undefined,
  }
  try {
    if (editingChannel.value) await api.updateChannel(editingChannel.value.id, body)
    else await api.createChannel(body)
    channelDialog.value = false
    await invalidate()
    toast.success(t('settings.notifications.channels.saved'))
  } catch (failure) {
    channelErrors.value = handleError(failure, CHANNEL_FIELDS)
  } finally {
    pending.value = false
  }
}

async function clearSecret(channel: Channel) {
  try {
    await api.updateChannel(channel.id, { name: channel.name, kind: channel.kind, language: channel.language, enabled: channel.enabled, secret: '' })
    await invalidate()
  } catch (failure) {
    handleError(failure)
  }
}

async function toggleChannel(channel: Channel, enabled: boolean) {
  try {
    await api.updateChannel(channel.id, { name: channel.name, kind: channel.kind, language: channel.language, enabled })
    await invalidate()
  } catch (failure) {
    handleError(failure)
  }
}

const testing = ref<string | null>(null)
async function testChannel(channel: Channel) {
  testing.value = channel.id
  try {
    const result = await api.testChannel(channel.id)
    if (result.ok) toast.success(t('settings.notifications.channels.testOk', { name: channel.name }))
    else toast.error(t('settings.notifications.channels.testFailed', { name: channel.name, error: result.error ?? '?' }))
    await invalidate()
  } catch (failure) {
    handleError(failure)
  } finally {
    testing.value = null
  }
}

async function removeChannel(channel: Channel) {
  try {
    await api.removeChannel(channel.id)
    await invalidate()
    toast.success(t('settings.notifications.channels.deleted', { name: channel.name }))
  } catch (failure) {
    handleError(failure)
  }
}

// --- Règles
const ruleDialog = ref(false)
const editingRule = ref<Rule | null>(null)
const ruleForm = reactive({ name: '', scope: 'all' as string, node_id: '', server_id: '', events: [] as string[], channels: [] as string[], enabled: true })
const ruleErrors = ref<FieldErrors>({})

function openRule(rule: Rule | null) {
  editingRule.value = rule
  ruleForm.name = rule?.name ?? ''
  ruleForm.scope = rule?.scope ?? 'all'
  ruleForm.node_id = rule?.node_id ?? ''
  ruleForm.server_id = rule?.server_id ?? ''
  ruleForm.events = [...(rule?.events ?? [])]
  ruleForm.channels = [...(rule?.channels ?? [])]
  ruleForm.enabled = rule?.enabled ?? true
  ruleErrors.value = {}
  ruleDialog.value = true
}

function toggle(list: string[], value: string, on: boolean) {
  const index = list.indexOf(value)
  if (on && index === -1) list.push(value)
  if (!on && index !== -1) list.splice(index, 1)
}

async function saveRule() {
  pending.value = true
  ruleErrors.value = {}
  const body: RuleRequest = {
    name: ruleForm.name,
    scope: ruleForm.scope,
    node_id: ruleForm.scope === 'node' ? ruleForm.node_id || null : null,
    server_id: ruleForm.scope === 'server' ? ruleForm.server_id || null : null,
    events: ruleForm.events,
    channels: ruleForm.channels,
    enabled: ruleForm.enabled,
  }
  try {
    if (editingRule.value) await api.updateRule(editingRule.value.id, body)
    else await api.createRule(body)
    ruleDialog.value = false
    await invalidate()
    toast.success(t('settings.notifications.rules.saved'))
  } catch (failure) {
    ruleErrors.value = handleError(failure, RULE_FIELDS)
  } finally {
    pending.value = false
  }
}

async function removeRule(rule: Rule) {
  try {
    await api.removeRule(rule.id)
    await invalidate()
    toast.success(t('settings.notifications.rules.deleted', { name: rule.name }))
  } catch (failure) {
    handleError(failure)
  }
}

function scopeLabel(rule: Rule): string {
  if (rule.scope === 'node') return t('settings.notifications.rules.scopeNode', { name: nodeName(rule.node_id) })
  if (rule.scope === 'server') return t('settings.notifications.rules.scopeServer', { name: serverName(rule.server_id) })
  return t('settings.notifications.rules.scopeAll')
}

const statusVariant = (status: string) => (status === 'sent' ? 'info' : status === 'pending' ? 'secondary' : 'destructive')
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('settings.notifications.title')" />
    <div class="grid gap-6 p-6">
      <p v-if="!readable" class="text-sm text-muted-foreground">{{ t('settings.notifications.forbidden') }}</p>
      <template v-else>
        <p class="max-w-3xl text-sm text-muted-foreground">{{ t('settings.notifications.help') }}</p>

        <Card class="gap-0 py-0">
          <CardHeader class="flex flex-wrap items-center justify-between gap-3 px-5 py-4">
            <div class="grid gap-1">
              <CardTitle>{{ t('settings.notifications.channels.title') }}</CardTitle>
              <CardDescription>{{ t('settings.notifications.channels.help') }}</CardDescription>
            </div>
            <Button v-if="editable" size="sm" @click="openChannel(null)"><Plus />{{ t('settings.notifications.channels.add') }}</Button>
          </CardHeader>
          <CardContent class="px-0 pb-0">
            <div v-if="channelsPending" class="grid gap-2 p-5"><Skeleton class="h-8" /><Skeleton class="h-8" /></div>
            <Empty v-else-if="!channels?.length" :icon="Bell" class="border-0 shadow-none">{{ t('settings.notifications.channels.empty') }}</Empty>
            <ul v-else class="divide-y border-t">
              <li v-for="channel in channels" :key="channel.id" class="flex flex-wrap items-center gap-3 px-5 py-3" data-channel>
                <span class="grid min-w-0 flex-1 gap-0.5">
                  <span class="flex min-w-0 flex-wrap items-center gap-2">
                    <span class="truncate font-semibold">{{ channel.name }}</span>
                    <Badge variant="outline">{{ t(`settings.notifications.kinds.${channel.kind}`) }}</Badge>
                    <Badge variant="secondary">{{ channel.language.toUpperCase() }}</Badge>
                    <Badge v-if="channel.has_secret" variant="secondary">{{ t('settings.notifications.channels.signed') }}</Badge>
                  </span>
                  <span class="truncate font-mono text-xs text-muted-foreground">{{ channel.url_hint }}</span>
                </span>
                <Switch
                  :model-value="channel.enabled"
                  :disabled="!editable"
                  :aria-label="t('settings.notifications.channels.enabled', { name: channel.name })"
                  @update:model-value="(value) => toggleChannel(channel, value)"
                />
                <template v-if="editable">
                  <Button size="sm" variant="outline" :disabled="testing === channel.id" @click="testChannel(channel)">
                    <Send />{{ t('settings.notifications.channels.test') }}
                  </Button>
                  <Button size="icon" variant="ghost" :aria-label="t('settings.notifications.channels.edit', { name: channel.name })" @click="openChannel(channel)"><Pencil /></Button>
                  <Button size="icon" variant="ghost" :aria-label="t('settings.notifications.channels.delete', { name: channel.name })" @click="removeChannel(channel)"><Trash2 /></Button>
                </template>
              </li>
            </ul>
          </CardContent>
        </Card>

        <Card class="gap-0 py-0">
          <CardHeader class="flex flex-wrap items-center justify-between gap-3 px-5 py-4">
            <div class="grid gap-1">
              <CardTitle>{{ t('settings.notifications.rules.title') }}</CardTitle>
              <CardDescription>{{ t('settings.notifications.rules.help') }}</CardDescription>
            </div>
            <Button v-if="editable" size="sm" :disabled="!channels?.length" @click="openRule(null)"><Plus />{{ t('settings.notifications.rules.add') }}</Button>
          </CardHeader>
          <CardContent class="px-0 pb-0">
            <Empty v-if="!rules?.length" :icon="Bell" class="border-0 shadow-none">{{ t('settings.notifications.rules.empty') }}</Empty>
            <ul v-else class="divide-y border-t">
              <li v-for="rule in rules" :key="rule.id" class="flex flex-wrap items-center gap-3 px-5 py-3" data-rule>
                <span class="grid min-w-0 flex-1 gap-1">
                  <span class="flex min-w-0 flex-wrap items-center gap-2">
                    <span class="truncate font-semibold">{{ rule.name }}</span>
                    <Badge variant="outline">{{ scopeLabel(rule) }}</Badge>
                    <Badge v-if="!rule.enabled" variant="secondary">{{ t('settings.notifications.rules.disabled') }}</Badge>
                  </span>
                  <span class="flex flex-wrap gap-1">
                    <Badge v-for="event in rule.events" :key="event" variant="secondary">{{ eventLabel(event) }}</Badge>
                  </span>
                  <span class="text-xs text-muted-foreground">{{ rule.channels.map(channelName).join(', ') }}</span>
                </span>
                <template v-if="editable">
                  <Button size="icon" variant="ghost" :aria-label="t('settings.notifications.rules.edit', { name: rule.name })" @click="openRule(rule)"><Pencil /></Button>
                  <Button size="icon" variant="ghost" :aria-label="t('settings.notifications.rules.delete', { name: rule.name })" @click="removeRule(rule)"><Trash2 /></Button>
                </template>
              </li>
            </ul>
          </CardContent>
        </Card>

        <Card class="gap-0 py-0">
          <CardHeader class="px-5 py-4">
            <CardTitle>{{ t('settings.notifications.deliveries.title') }}</CardTitle>
            <CardDescription>{{ t('settings.notifications.deliveries.help') }}</CardDescription>
          </CardHeader>
          <CardContent class="px-0 pb-0">
            <Empty v-if="!deliveries?.length" :icon="Bell" class="border-0 shadow-none">{{ t('settings.notifications.deliveries.empty') }}</Empty>
            <Table v-else>
              <TableHeader>
                <TableRow>
                  <TableHead>{{ t('settings.notifications.deliveries.when') }}</TableHead>
                  <TableHead>{{ t('settings.notifications.deliveries.event') }}</TableHead>
                  <TableHead>{{ t('settings.notifications.deliveries.target') }}</TableHead>
                  <TableHead>{{ t('settings.notifications.deliveries.channel') }}</TableHead>
                  <TableHead>{{ t('settings.notifications.deliveries.status') }}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow v-for="delivery in deliveries" :key="delivery.id" data-delivery>
                  <TableCell class="whitespace-nowrap">{{ when(delivery.created_at) }}</TableCell>
                  <TableCell>{{ eventLabel(String((delivery.event as { kind?: string }).kind ?? '')) }}</TableCell>
                  <TableCell>{{ target(delivery as { event: Record<string, unknown> }) }}</TableCell>
                  <TableCell>{{ channelName(delivery.channel_id) }}</TableCell>
                  <TableCell>
                    <Badge :variant="statusVariant(delivery.status)" :title="delivery.last_error ?? undefined">
                      {{ t(`settings.notifications.deliveries.statuses.${delivery.status}`, { attempts: delivery.attempts }) }}
                    </Badge>
                    <span v-if="delivery.last_error" class="ml-2 font-mono text-xs text-muted-foreground">{{ delivery.last_error }}</span>
                  </TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      </template>
    </div>

    <Dialog :open="channelDialog" @update:open="(open) => (channelDialog = open)">
      <DialogContent class="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{{ editingChannel ? t('settings.notifications.channels.editTitle', { name: editingChannel.name }) : t('settings.notifications.channels.add') }}</DialogTitle>
          <DialogDescription>{{ t('settings.notifications.channels.dialogHelp') }}</DialogDescription>
        </DialogHeader>
        <form class="grid gap-4" @submit.prevent="saveChannel">
          <div class="grid gap-1.5">
            <Label for="channel-name">{{ t('settings.notifications.fields.name') }}</Label>
            <Input id="channel-name" v-model="channelForm.name" :aria-invalid="!!channelErrors.name" />
            <p v-if="channelErrors.name" class="text-xs text-destructive">{{ channelErrors.name }}</p>
          </div>
          <div class="grid gap-1.5">
            <Label for="channel-kind">{{ t('settings.notifications.fields.kind') }}</Label>
            <Select v-model="channelForm.kind">
              <SelectTrigger id="channel-kind" class="w-full"><SelectValue>{{ t(`settings.notifications.kinds.${channelForm.kind}`) }}</SelectValue></SelectTrigger>
              <SelectContent>
                <SelectItem v-for="kind in KINDS" :key="kind" :value="kind">{{ t(`settings.notifications.kinds.${kind}`) }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div v-if="channelForm.kind === 'email'" class="grid gap-1.5">
            <Label for="channel-recipients">{{ t('settings.notifications.fields.recipients') }}</Label>
            <Textarea id="channel-recipients" v-model="channelForm.recipients" rows="3" class="font-mono" :placeholder="editingChannel ? t('settings.notifications.fields.unchanged') : 'ops@example.com'" :aria-invalid="!!channelErrors.recipients" />
            <p v-if="channelErrors.recipients" class="text-xs text-destructive">{{ channelErrors.recipients }}</p>
            <p v-else class="text-xs text-muted-foreground">{{ t('settings.notifications.fields.recipientsHelp') }}</p>
          </div>
          <div v-else class="grid gap-1.5">
            <Label for="channel-url">{{ t('settings.notifications.fields.url') }}</Label>
            <Input id="channel-url" v-model="channelForm.url" type="url" class="font-mono" :placeholder="editingChannel ? t('settings.notifications.fields.unchanged') : 'https://'" :aria-invalid="!!channelErrors.url" />
            <p v-if="channelErrors.url" class="text-xs text-destructive">{{ channelErrors.url }}</p>
            <p v-else class="text-xs text-muted-foreground">{{ t('settings.notifications.fields.urlHelp') }}</p>
          </div>
          <div v-if="channelForm.kind === 'webhook'" class="grid gap-1.5">
            <Label for="channel-secret">{{ t('settings.notifications.fields.secret') }}</Label>
            <Input id="channel-secret" v-model="channelForm.secret" type="password" autocomplete="off" :placeholder="editingChannel?.has_secret ? t('settings.notifications.fields.unchanged') : ''" />
            <p class="text-xs text-muted-foreground">{{ t('settings.notifications.fields.secretHelp') }}</p>
            <Button v-if="editingChannel?.has_secret" type="button" variant="link" size="sm" class="justify-self-start px-0" @click="editingChannel && clearSecret(editingChannel)">
              {{ t('settings.notifications.fields.clearSecret') }}
            </Button>
          </div>
          <div class="grid gap-1.5">
            <Label for="channel-language">{{ t('settings.notifications.fields.language') }}</Label>
            <Select v-model="channelForm.language">
              <SelectTrigger id="channel-language" class="w-full"><SelectValue>{{ t(`settings.notifications.languages.${channelForm.language}`) }}</SelectValue></SelectTrigger>
              <SelectContent>
                <SelectItem v-for="language in LANGUAGES" :key="language" :value="language">{{ t(`settings.notifications.languages.${language}`) }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div class="flex items-center gap-2">
            <Switch id="channel-enabled" :model-value="channelForm.enabled" @update:model-value="(value) => (channelForm.enabled = value)" />
            <Label for="channel-enabled">{{ t('settings.notifications.fields.enabled') }}</Label>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" @click="channelDialog = false">{{ t('common.cancel') }}</Button>
            <Button type="submit" :disabled="pending">{{ t('common.save') }}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>

    <Dialog :open="ruleDialog" @update:open="(open) => (ruleDialog = open)">
      <DialogContent class="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{{ editingRule ? t('settings.notifications.rules.editTitle', { name: editingRule.name }) : t('settings.notifications.rules.add') }}</DialogTitle>
          <DialogDescription>{{ t('settings.notifications.rules.dialogHelp') }}</DialogDescription>
        </DialogHeader>
        <form class="grid gap-4" @submit.prevent="saveRule">
          <div class="grid gap-1.5">
            <Label for="rule-name">{{ t('settings.notifications.fields.name') }}</Label>
            <Input id="rule-name" v-model="ruleForm.name" :aria-invalid="!!ruleErrors.name" />
            <p v-if="ruleErrors.name" class="text-xs text-destructive">{{ ruleErrors.name }}</p>
          </div>
          <div class="grid gap-1.5">
            <Label for="rule-scope">{{ t('settings.notifications.fields.scope') }}</Label>
            <Select v-model="ruleForm.scope">
              <SelectTrigger id="rule-scope" class="w-full"><SelectValue>{{ t(`settings.notifications.scopes.${ruleForm.scope}`) }}</SelectValue></SelectTrigger>
              <SelectContent>
                <SelectItem v-for="scope in SCOPES" :key="scope" :value="scope">{{ t(`settings.notifications.scopes.${scope}`) }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div v-if="ruleForm.scope === 'node'" class="grid gap-1.5">
            <Label for="rule-node">{{ t('settings.notifications.fields.node') }}</Label>
            <Select v-model="ruleForm.node_id">
              <SelectTrigger id="rule-node" class="w-full" :aria-invalid="!!ruleErrors.node_id"><SelectValue>{{ ruleForm.node_id ? nodeName(ruleForm.node_id) : '' }}</SelectValue></SelectTrigger>
              <SelectContent>
                <SelectItem v-for="node in nodes ?? []" :key="node.id" :value="node.id">{{ node.name }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div v-else-if="ruleForm.scope === 'server'" class="grid gap-1.5">
            <Label for="rule-server">{{ t('settings.notifications.fields.server') }}</Label>
            <Select v-model="ruleForm.server_id">
              <SelectTrigger id="rule-server" class="w-full" :aria-invalid="!!ruleErrors.server_id"><SelectValue>{{ ruleForm.server_id ? serverName(ruleForm.server_id) : '' }}</SelectValue></SelectTrigger>
              <SelectContent>
                <SelectItem v-for="server in servers ?? []" :key="server.id" :value="server.id">{{ server.name }}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <fieldset class="grid gap-2">
            <legend class="text-sm font-medium">{{ t('settings.notifications.fields.events') }}</legend>
            <label v-for="event in events ?? []" :key="event" class="flex items-center gap-2 text-sm">
              <Checkbox :model-value="ruleForm.events.includes(event)" @update:model-value="(value) => toggle(ruleForm.events, event, value === true)" />
              {{ eventLabel(event) }}
            </label>
            <p v-if="ruleErrors.events" class="text-xs text-destructive">{{ ruleErrors.events }}</p>
          </fieldset>
          <fieldset class="grid gap-2">
            <legend class="text-sm font-medium">{{ t('settings.notifications.fields.channels') }}</legend>
            <label v-for="channel in channels ?? []" :key="channel.id" class="flex items-center gap-2 text-sm">
              <Checkbox :model-value="ruleForm.channels.includes(channel.id)" @update:model-value="(value) => toggle(ruleForm.channels, channel.id, value === true)" />
              {{ channel.name }}
            </label>
            <p v-if="ruleErrors.channels" class="text-xs text-destructive">{{ ruleErrors.channels }}</p>
          </fieldset>
          <div class="flex items-center gap-2">
            <Switch id="rule-enabled" :model-value="ruleForm.enabled" @update:model-value="(value) => (ruleForm.enabled = value)" />
            <Label for="rule-enabled">{{ t('settings.notifications.fields.enabled') }}</Label>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" @click="ruleDialog = false">{{ t('common.cancel') }}</Button>
            <Button type="submit" :disabled="pending">{{ t('common.save') }}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  </div>
</template>
