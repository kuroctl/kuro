<script setup lang="ts">
// Vérification en deux étapes du compte (ADR-0007) : activation, changement d'appareil, codes de
// secours, désactivation si la plateforme ne l'exige pas. Chaque action redemande le mot de passe,
// et un code actuel (ou de secours) quand le TOTP est actif : l'API en reste seule juge.
import { ShieldCheck, ShieldOff } from '@lucide/vue'
import QRCode from 'qrcode'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import RecoveryCodes from '@/features/auth/components/RecoveryCodes.vue'
import TotpCodeInput from '@/features/auth/components/TotpCodeInput.vue'
import type { Schemas } from '@/shared/api/types'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { useAccountApi } from '../account.api'
import { useInvalidateMe } from '../account.queries'

const props = defineProps<{ profile: Schemas['ProfileDto'] }>()

/** En deçà, le compte est invité à régénérer ses codes. */
const LOW_CODES = 3

type Mode = 'setup' | 'regenerate' | 'disable'
type Step = 'reauth' | 'scan' | 'codes'

const { t } = useI18n()
const api = useAccountApi()
const invalidateMe = useInvalidateMe()
const { handleError } = useApiError()

const enabled = computed(() => props.profile.user.totp_enabled)
const remaining = computed(() => props.profile.recovery_codes_remaining)

const mode = ref<Mode | null>(null)
const step = ref<Step>('reauth')
const pending = ref(false)
const fieldErrors = ref<FieldErrors>({})
const password = ref('')
const code = ref('')
const newCode = ref('')
const setup = ref<{ secret: string; qr: string } | null>(null)
const codes = ref<string[]>([])

const title = computed(() => {
  if (step.value === 'scan') return t('account.totp.scanTitle')
  if (step.value === 'codes') return t('auth.recoveryCodes.title')
  return mode.value === 'disable' ? t('account.totp.disableTitle') : t('account.totp.reauthTitle')
})
const description = computed(() => {
  if (step.value === 'scan') return t('account.totp.scanHelp')
  if (step.value === 'codes') return ''
  if (mode.value === 'disable') return t('account.totp.disableHelp')
  return enabled.value ? t('account.totp.reauthHelpCode') : t('account.totp.reauthHelp')
})

function open(next: Mode) {
  mode.value = next
  step.value = 'reauth'
  password.value = ''
  code.value = ''
  newCode.value = ''
  setup.value = null
  codes.value = []
  fieldErrors.value = {}
}

function close() {
  mode.value = null
}

async function submitReauth() {
  pending.value = true
  fieldErrors.value = {}
  const body = { password: password.value, code: enabled.value ? code.value : null }
  try {
    if (mode.value === 'setup') {
      const totp = await api.startTotpSetup(body)
      setup.value = { secret: totp.secret, qr: await QRCode.toDataURL(totp.otpauth_uri, { margin: 1 }) }
      step.value = 'scan'
    } else if (mode.value === 'regenerate') {
      codes.value = (await api.regenerateRecoveryCodes(body)).recovery_codes
      step.value = 'codes'
      toast.success(t('account.totp.regeneratedToast'))
    } else {
      await api.disableTotp(body)
      toast.success(t('account.totp.disabledToast'))
      close()
    }
    await invalidateMe()
  } catch (error) {
    fieldErrors.value = handleError(error, ['password', 'code'])
    code.value = ''
  } finally {
    pending.value = false
  }
}

async function submitScan() {
  // Déjà en cours : le sixième chiffre et le bouton peuvent valider ensemble.
  if (pending.value) return
  pending.value = true
  fieldErrors.value = {}
  const replaced = enabled.value
  try {
    codes.value = (await api.confirmTotp({ code: newCode.value })).recovery_codes
    step.value = 'codes'
    toast.success(replaced ? t('account.totp.replacedToast') : t('account.totp.enabledToast'))
    await invalidateMe()
  } catch (error) {
    fieldErrors.value = handleError(error, ['code'])
    newCode.value = ''
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle class="flex items-center gap-2">
        <ShieldCheck v-if="enabled" class="size-4 text-success" />
        <ShieldOff v-else class="size-4 text-warning" />
        {{ t('account.totp.title') }}
        <Badge :variant="enabled ? 'secondary' : 'outline'" class="ml-auto">
          {{ enabled ? t('account.totp.enabled') : t('account.totp.disabled') }}
        </Badge>
      </CardTitle>
      <CardDescription>
        {{ enabled ? t('account.totp.helpEnabled') : t('account.totp.helpDisabled') }}
        <template v-if="profile.totp_required">{{ t('account.totp.required') }}</template>
      </CardDescription>
    </CardHeader>
    <CardContent class="grid gap-4 text-sm">
      <template v-if="enabled">
        <p :class="remaining <= LOW_CODES ? 'text-warning' : 'text-muted-foreground'">
          {{ t('account.totp.remaining', remaining) }}
          <template v-if="remaining <= LOW_CODES">{{ t('account.totp.lowCodes') }}</template>
        </p>
        <div class="flex flex-wrap gap-2">
          <Button variant="outline" @click="open('setup')">{{ t('account.totp.replace') }}</Button>
          <Button variant="outline" @click="open('regenerate')">{{ t('account.totp.regenerate') }}</Button>
          <Button v-if="!profile.totp_required" variant="ghost" class="text-destructive" @click="open('disable')">
            {{ t('account.totp.disable') }}
          </Button>
        </div>
      </template>
      <Button v-else class="self-start" @click="open('setup')">{{ t('account.totp.enable') }}</Button>
    </CardContent>
  </Card>

  <Dialog :open="mode !== null" @update:open="(value) => !value && close()">
    <DialogContent class="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>{{ title }}</DialogTitle>
        <DialogDescription v-if="description">{{ description }}</DialogDescription>
      </DialogHeader>

      <form v-if="step === 'reauth'" class="grid gap-4" @submit.prevent="submitReauth">
        <div class="grid gap-2">
          <Label for="totp-password">{{ t('account.totp.password') }}</Label>
          <Input id="totp-password" v-model="password" type="password" autocomplete="current-password" required />
          <p v-if="fieldErrors.password" class="text-sm text-destructive">{{ fieldErrors.password }}</p>
        </div>
        <div v-if="enabled" class="grid gap-2">
          <Label for="totp-code">{{ t('account.totp.code') }}</Label>
          <Input id="totp-code" v-model="code" autocomplete="one-time-code" spellcheck="false" class="font-mono" required />
          <p v-if="fieldErrors.code" class="text-sm text-destructive">{{ fieldErrors.code }}</p>
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" @click="close">{{ t('common.cancel') }}</Button>
          <Button type="submit" :variant="mode === 'disable' ? 'destructive' : 'default'" :disabled="pending">
            {{ mode === 'disable' ? t('account.totp.disable') : t('account.totp.continue') }}
          </Button>
        </DialogFooter>
      </form>

      <form v-else-if="step === 'scan' && setup" class="grid gap-4" @submit.prevent="submitScan">
        <div class="flex flex-col items-center gap-3">
          <img :src="setup.qr" :alt="t('auth.totpSetup.qrAlt')" class="size-44 rounded-lg bg-white p-2">
          <div class="w-full text-center">
            <p class="text-xs text-muted-foreground">{{ t('auth.totpSetup.manual') }}</p>
            <code class="break-all font-mono text-xs">{{ setup.secret }}</code>
          </div>
        </div>
        <div class="grid gap-2">
          <Label for="totp-new-code">{{ t('account.totp.newCode') }}</Label>
          <TotpCodeInput
            id="totp-new-code"
            v-model="newCode"
            :disabled="pending"
            :invalid="!!fieldErrors.code"
            @complete="submitScan"
          />
          <p v-if="fieldErrors.code" class="text-sm text-destructive">{{ fieldErrors.code }}</p>
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" @click="close">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="pending || newCode.length !== 6">{{ t('account.totp.confirm') }}</Button>
        </DialogFooter>
      </form>

      <RecoveryCodes v-else-if="step === 'codes'" :codes="codes" :account="profile.user.email" @done="close" />
    </DialogContent>
  </Dialog>
</template>
