<script setup lang="ts">
// Réglages SMTP (ADR-0046) : hôte, port, chiffrement, identifiants, expéditeur ; e-mail de test
// à l'admin connecté. Le mot de passe est chiffré par l'API et jamais réaffiché : le champ reste
// vide une fois le serveur en place. Partagé par Paramètres › E-mail et l'assistant de mise en route.
import { KeyRound, Send, Trash2 } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { type MailSettings, type MailSettingsRequest, useMailApi } from '@/features/mail/mail.api'
import { useInvalidateMail } from '@/features/mail/mail.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

const props = defineProps<{
  settings: MailSettings
  editable: boolean
}>()
const emit = defineEmits<{ saved: []; removed: [] }>()

const FIELDS = ['host', 'port', 'security', 'username', 'password', 'from_address', 'from_name'] as const

const { t, locale } = useI18n()
const api = useMailApi()
const invalidate = useInvalidateMail()
const { handleError } = useApiError()

const form = reactive({ host: '', port: 587, security: 'starttls' as string, username: '', password: '', from_address: '', from_name: '' })
/** « Aucun » n'existe que si l'API l'accepte (`mail.allow_insecure`, boîte de dev). */
const securities = computed(() => (props.settings.insecure_allowed ? ['starttls', 'tls', 'none'] : ['starttls', 'tls']))
const errors = ref<FieldErrors>({})
const pending = ref<'save' | 'test' | 'remove'>()

function load(settings: MailSettings) {
  form.host = settings.host ?? ''
  form.port = settings.port ?? 587
  form.security = settings.security ?? 'starttls'
  form.username = settings.username ?? ''
  form.password = ''
  form.from_address = settings.from_address ?? ''
  form.from_name = settings.from_name ?? ''
}
watch(() => props.settings, load, { immediate: true })

const since = computed(() =>
  props.settings.updated_at
    ? new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(props.settings.updated_at))
    : undefined,
)

async function save() {
  pending.value = 'save'
  errors.value = {}
  const body: MailSettingsRequest = {
    host: form.host.trim(),
    port: Number(form.port),
    security: form.security,
    username: form.username.trim() || null,
    from_address: form.from_address.trim(),
    from_name: form.from_name.trim() || null,
  }
  // Absent : le mot de passe enregistré est gardé ; vide quand on veut le retirer.
  if (form.password) body.password = form.password
  try {
    await api.set(body)
    form.password = ''
    toast.success(t('mail.saved'))
    await invalidate()
    emit('saved')
  } catch (failure) {
    errors.value = handleError(failure, FIELDS)
  } finally {
    pending.value = undefined
  }
}

async function test() {
  pending.value = 'test'
  try {
    const result = await api.test()
    toast.success(t('mail.testOk', { to: result.to }))
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = undefined
  }
}

async function remove() {
  pending.value = 'remove'
  try {
    await api.remove()
    toast.success(t('mail.removed'))
    await invalidate()
    emit('removed')
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = undefined
  }
}
</script>

<template>
  <form class="grid gap-4" @submit.prevent="save">
    <p v-if="since" class="text-sm text-muted-foreground">{{ t('mail.since', { date: since }) }}</p>
    <div class="grid gap-4 sm:grid-cols-[1fr_8rem]">
      <div class="grid gap-1.5">
        <Label for="mail-host">{{ t('mail.fields.host') }}</Label>
        <Input id="mail-host" v-model="form.host" autocomplete="off" spellcheck="false" placeholder="smtp.example.com" :disabled="!editable" :aria-invalid="!!errors.host" />
        <p v-if="errors.host" class="text-sm text-destructive">{{ errors.host }}</p>
      </div>
      <div class="grid gap-1.5">
        <Label for="mail-port">{{ t('mail.fields.port') }}</Label>
        <Input id="mail-port" v-model.number="form.port" type="number" min="1" max="65535" :disabled="!editable" :aria-invalid="!!errors.port" />
        <p v-if="errors.port" class="text-sm text-destructive">{{ errors.port }}</p>
      </div>
    </div>
    <div class="grid gap-1.5">
      <Label for="mail-security">{{ t('mail.fields.security') }}</Label>
      <Select v-model="form.security" :disabled="!editable">
        <SelectTrigger id="mail-security" class="w-full"><SelectValue>{{ t(`mail.securities.${form.security}`) }}</SelectValue></SelectTrigger>
        <SelectContent>
          <SelectItem v-for="security in securities" :key="security" :value="security">{{ t(`mail.securities.${security}`) }}</SelectItem>
        </SelectContent>
      </Select>
      <p v-if="errors.security" class="text-sm text-destructive">{{ errors.security }}</p>
    </div>
    <div class="grid gap-4 sm:grid-cols-2">
      <div class="grid gap-1.5">
        <Label for="mail-username">{{ t('mail.fields.username') }}</Label>
        <Input id="mail-username" v-model="form.username" autocomplete="off" spellcheck="false" :disabled="!editable" :aria-invalid="!!errors.username" />
        <p v-if="errors.username" class="text-sm text-destructive">{{ errors.username }}</p>
      </div>
      <div class="grid gap-1.5">
        <Label for="mail-password">{{ settings.has_password ? t('mail.fields.replacePassword') : t('mail.fields.password') }}</Label>
        <Input id="mail-password" v-model="form.password" type="password" autocomplete="new-password" spellcheck="false" :disabled="!editable" :aria-invalid="!!errors.password" />
        <p v-if="errors.password" class="text-sm text-destructive">{{ errors.password }}</p>
        <p v-else-if="settings.has_password" class="text-xs text-muted-foreground">{{ t('mail.fields.passwordKept') }}</p>
      </div>
    </div>
    <div class="grid gap-4 sm:grid-cols-2">
      <div class="grid gap-1.5">
        <Label for="mail-from">{{ t('mail.fields.fromAddress') }}</Label>
        <Input id="mail-from" v-model="form.from_address" type="email" autocomplete="off" spellcheck="false" :disabled="!editable" :aria-invalid="!!errors.from_address" />
        <p v-if="errors.from_address" class="text-sm text-destructive">{{ errors.from_address }}</p>
      </div>
      <div class="grid gap-1.5">
        <Label for="mail-from-name">{{ t('mail.fields.fromName') }}</Label>
        <Input id="mail-from-name" v-model="form.from_name" autocomplete="off" :placeholder="t('mail.fields.fromNameHelp')" :disabled="!editable" :aria-invalid="!!errors.from_name" />
        <p v-if="errors.from_name" class="text-sm text-destructive">{{ errors.from_name }}</p>
      </div>
    </div>
    <div v-if="editable" class="flex flex-wrap justify-end gap-2">
      <Button v-if="settings.configured" type="button" variant="outline" :disabled="!!pending" @click="remove">
        <Trash2 />{{ t('mail.remove') }}
      </Button>
      <Button v-if="settings.configured" type="button" variant="secondary" :disabled="!!pending" @click="test">
        <Send />{{ t('mail.test') }}
      </Button>
      <Button type="submit" :disabled="!!pending || !form.host.trim() || !form.from_address.trim()">
        <KeyRound />{{ t('mail.save') }}
      </Button>
    </div>
  </form>
</template>
