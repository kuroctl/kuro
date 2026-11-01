<script setup lang="ts">
// Connexion en étapes : identifiants, nouveau mot de passe si le sien a été choisi par un autre
// admin, puis code TOTP (ou code de secours) ou configuration du TOTP, suivie de ses codes de
// secours.
import QRCode from 'qrcode'
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Schemas } from '@/shared/api/types'
import { useApiError, type FieldErrors } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { useAuthApi } from '../auth.api'
import RecoveryCodes from './RecoveryCodes.vue'
import TotpCodeInput from './TotpCodeInput.vue'
import { useSessionStore } from '../session.store'

const props = defineProps<{
  /** Réponse déjà obtenue (invitation acceptée) : le flux reprend à l'étape qu'elle indique. */
  resume?: Schemas['LoginResponse']
}>()
const emit = defineEmits<{ authenticated: [] }>()

const { t } = useI18n()
const { login, changeRequiredPassword, verifyTotp, startTotpSetup, confirmTotp } = useAuthApi()
const { handleError } = useApiError()
const session = useSessionStore()

type Step = 'credentials' | 'password' | 'totp' | 'setup' | 'codes'
const step = ref<Step>('credentials')
const pending = ref(false)
const fieldErrors = ref<FieldErrors>({})

const email = ref('')
const password = ref('')
const newPassword = ref('')
const confirmation = ref('')
const code = ref('')
const challenge = ref('')
const setup = ref<{ secret: string; qr: string } | null>(null)
/** Code de secours saisi à la place du code de l'application. */
const recovery = ref(false)
/** Codes de secours délivrés à l'activation, et session à ouvrir une fois qu'ils sont gardés. */
const recoveryCodes = ref<string[]>([])
const pendingToken = ref('')

function finish(accessToken: string) {
  session.open(accessToken)
  emit('authenticated')
}

/** Étape suivante, après les identifiants ou le nouveau mot de passe. */
async function next(result: Schemas['LoginResponse']) {
  if (result.status === 'authenticated') return finish(result.access_token)

  challenge.value = result.challenge
  if (result.status === 'password_change_required') {
    step.value = 'password'
  } else if (result.status === 'totp_required') {
    recovery.value = false
    step.value = 'totp'
  } else {
    const totp = await startTotpSetup({ challenge: challenge.value })
    setup.value = { secret: totp.secret, qr: await QRCode.toDataURL(totp.otpauth_uri, { margin: 1 }) }
    step.value = 'setup'
  }
}

onMounted(() => {
  if (props.resume) void next(props.resume)
})

async function submitCredentials() {
  pending.value = true
  fieldErrors.value = {}
  try {
    await next(await login({ email: email.value, password: password.value }))
  } catch (error) {
    fieldErrors.value = handleError(error, ['email', 'password'])
  } finally {
    pending.value = false
  }
}

async function submitPassword() {
  fieldErrors.value = {}
  if (newPassword.value !== confirmation.value) {
    fieldErrors.value = { confirmation: t('auth.newPassword.mismatch') }
    return
  }
  pending.value = true
  try {
    await next(await changeRequiredPassword({ challenge: challenge.value, new_password: newPassword.value }))
  } catch (error) {
    fieldErrors.value = handleError(error, ['new_password'])
  } finally {
    pending.value = false
  }
}

function restart() {
  step.value = 'credentials'
  password.value = ''
  newPassword.value = ''
  confirmation.value = ''
  fieldErrors.value = {}
}

function toggleRecovery() {
  recovery.value = !recovery.value
  code.value = ''
}

async function submitCode() {
  // Déjà en cours : le sixième chiffre et le bouton peuvent valider ensemble.
  if (pending.value) return
  pending.value = true
  try {
    const body = { challenge: challenge.value, code: code.value }
    if (step.value === 'totp') {
      finish((await verifyTotp(body)).access_token)
    } else {
      const confirmed = await confirmTotp(body)
      pendingToken.value = confirmed.access_token
      recoveryCodes.value = confirmed.recovery_codes
      step.value = 'codes'
    }
  } catch (error) {
    handleError(error)
    code.value = ''
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Card class="w-full max-w-sm border-border/60 shadow-2xl shadow-black/40">
    <template v-if="step === 'credentials'">
      <CardHeader>
        <CardTitle class="text-xl">{{ t('auth.title') }}</CardTitle>
        <CardDescription>{{ t('auth.subtitle') }}</CardDescription>
      </CardHeader>
      <CardContent>
        <form class="flex flex-col gap-4" @submit.prevent="submitCredentials">
          <div class="flex flex-col gap-2">
            <Label for="email">{{ t('auth.email') }}</Label>
            <Input id="email" v-model="email" type="email" autocomplete="username" required />
            <p v-if="fieldErrors.email" class="text-sm text-destructive">{{ fieldErrors.email }}</p>
          </div>
          <div class="flex flex-col gap-2">
            <div class="flex items-baseline justify-between gap-2">
              <Label for="password">{{ t('auth.password') }}</Label>
              <RouterLink to="/password/forgot" class="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline">
                {{ t('passwordReset.forgotLink') }}
              </RouterLink>
            </div>
            <Input id="password" v-model="password" type="password" autocomplete="current-password" required />
            <p v-if="fieldErrors.password" class="text-sm text-destructive">{{ fieldErrors.password }}</p>
          </div>
          <Button type="submit" :disabled="pending">{{ t('auth.submit') }}</Button>
        </form>
      </CardContent>
    </template>

    <template v-else-if="step === 'password'">
      <CardHeader>
        <CardTitle class="text-xl">{{ t('auth.newPassword.title') }}</CardTitle>
        <CardDescription>{{ t('auth.newPassword.subtitle') }}</CardDescription>
      </CardHeader>
      <CardContent>
        <form class="flex flex-col gap-4" @submit.prevent="submitPassword">
          <div class="flex flex-col gap-2">
            <Label for="new-password">{{ t('auth.newPassword.password') }}</Label>
            <Input
              id="new-password"
              v-model="newPassword"
              type="password"
              autocomplete="new-password"
              required
              :aria-invalid="!!fieldErrors.new_password"
            />
            <p v-if="fieldErrors.new_password" class="text-sm text-destructive">{{ fieldErrors.new_password }}</p>
            <p v-else class="text-xs text-muted-foreground">{{ t('auth.newPassword.help') }}</p>
          </div>
          <div class="flex flex-col gap-2">
            <Label for="confirmation">{{ t('auth.newPassword.confirmation') }}</Label>
            <Input
              id="confirmation"
              v-model="confirmation"
              type="password"
              autocomplete="new-password"
              required
              :aria-invalid="!!fieldErrors.confirmation"
            />
            <p v-if="fieldErrors.confirmation" class="text-sm text-destructive">{{ fieldErrors.confirmation }}</p>
          </div>
          <Button type="submit" :disabled="pending">{{ t('auth.newPassword.submit') }}</Button>
          <Button type="button" variant="ghost" @click="restart">{{ t('common.back') }}</Button>
        </form>
      </CardContent>
    </template>

    <template v-else-if="step === 'codes'">
      <CardHeader>
        <CardTitle class="text-xl">{{ t('auth.recoveryCodes.title') }}</CardTitle>
      </CardHeader>
      <CardContent>
        <RecoveryCodes :codes="recoveryCodes" :account="email" @done="finish(pendingToken)" />
      </CardContent>
    </template>

    <template v-else>
      <CardHeader>
        <CardTitle class="text-xl">
          {{ step === 'setup' ? t('auth.totpSetup.title') : recovery ? t('auth.totp.recoveryTitle') : t('auth.totp.title') }}
        </CardTitle>
        <CardDescription>
          {{ step === 'setup' ? t('auth.totpSetup.subtitle') : recovery ? t('auth.totp.recoverySubtitle') : t('auth.totp.subtitle') }}
        </CardDescription>
      </CardHeader>
      <CardContent class="flex flex-col gap-4">
        <div v-if="step === 'setup' && setup" class="flex flex-col items-center gap-3">
          <img :src="setup.qr" :alt="t('auth.totpSetup.qrAlt')" class="size-44 rounded-lg bg-white p-2">
          <div class="w-full text-center">
            <p class="text-xs text-muted-foreground">{{ t('auth.totpSetup.manual') }}</p>
            <code class="break-all font-mono text-xs">{{ setup.secret }}</code>
          </div>
        </div>
        <form class="flex flex-col gap-4" @submit.prevent="submitCode">
          <div v-if="recovery" class="flex flex-col gap-2">
            <Label for="recovery-code">{{ t('auth.totp.recoveryCode') }}</Label>
            <Input
              id="recovery-code"
              v-model="code"
              autocomplete="off"
              spellcheck="false"
              class="text-center font-mono"
              required
            />
          </div>
          <div v-else class="flex flex-col gap-2">
            <Label for="code">{{ t('auth.totp.code') }}</Label>
            <TotpCodeInput id="code" v-model="code" :disabled="pending" @complete="submitCode" />
          </div>
          <Button type="submit" :disabled="pending || (recovery ? !code.trim() : code.length !== 6)">
            {{ step === 'totp' ? t('auth.totp.submit') : t('auth.totpSetup.submit') }}
          </Button>
          <Button v-if="step === 'totp'" type="button" variant="link" size="sm" @click="toggleRecovery">
            {{ recovery ? t('auth.totp.useApp') : t('auth.totp.useRecovery') }}
          </Button>
          <Button type="button" variant="ghost" @click="step = 'credentials'">{{ t('common.back') }}</Button>
        </form>
      </CardContent>
    </template>
  </Card>
</template>
