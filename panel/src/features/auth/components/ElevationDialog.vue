<script setup lang="ts">
// Confirmation d'identité avant une action protégée (ADR-0050) : code de l'application (ou code
// de secours) si le TOTP est configuré, mot de passe sinon. L'API a refusé la requête par
// `403 elevation_required`, qui sera rejouée.
//
// Montée par `App.vue` seulement pendant une demande : son portail s'ajoute alors en dernier
// dans `<body>`, au-dessus de la fenêtre déjà ouverte qui a lancé l'action (confirmation de
// suppression…), sans `z-index` à forcer. Chaque demande repart aussi d'une fenêtre vierge.
import { ShieldCheck } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { useAccountApi } from '@/features/account/account.api'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { useElevationStore } from '../elevation.store'
import TotpCodeInput from './TotpCodeInput.vue'

const { t } = useI18n()
const store = useElevationStore()
const api = useAccountApi()
const { handleError } = useApiError()

const password = ref('')
const code = ref('')
/** Code de secours saisi à la place du code de l'application. */
const recovery = ref(false)
const pending = ref(false)
const fieldErrors = ref<FieldErrors>({})

const totp = computed(() => store.method === 'totp')
const ready = computed(() => {
  if (!totp.value) return password.value.length > 0
  return recovery.value ? code.value.trim().length > 0 : code.value.length === 6
})

function toggleRecovery() {
  recovery.value = !recovery.value
  code.value = ''
  fieldErrors.value = {}
}

async function submit() {
  // Déjà en cours : le sixième chiffre et le bouton peuvent valider ensemble.
  if (pending.value || !ready.value) return
  pending.value = true
  fieldErrors.value = {}
  try {
    await api.elevate(totp.value ? { code: code.value } : { password: password.value })
    store.settle(true)
  } catch (error) {
    fieldErrors.value = handleError(error, ['password', 'code'])
    code.value = ''
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Dialog open @update:open="(open) => !open && store.settle(false)">
    <DialogContent class="sm:max-w-sm">
      <DialogHeader>
        <DialogTitle class="flex items-center gap-2"><ShieldCheck class="size-4" />{{ t('elevation.title') }}</DialogTitle>
        <DialogDescription>
          {{ !totp ? t('elevation.helpPassword') : recovery ? t('elevation.helpRecovery') : t('elevation.helpTotp') }}
        </DialogDescription>
      </DialogHeader>
      <form class="grid gap-4" @submit.prevent="submit">
        <div v-if="!totp" class="grid gap-2">
          <Label for="elevation-password">{{ t('elevation.password') }}</Label>
          <Input id="elevation-password" v-model="password" type="password" autocomplete="current-password" autofocus required />
          <p v-if="fieldErrors.password" class="text-sm text-destructive">{{ fieldErrors.password }}</p>
        </div>
        <div v-else-if="recovery" class="grid gap-2">
          <Label for="elevation-recovery">{{ t('auth.totp.recoveryCode') }}</Label>
          <Input id="elevation-recovery" v-model="code" autocomplete="off" spellcheck="false" class="text-center font-mono" autofocus required />
          <p v-if="fieldErrors.code" class="text-sm text-destructive">{{ fieldErrors.code }}</p>
        </div>
        <div v-else class="grid gap-2">
          <Label for="elevation-code">{{ t('auth.totp.code') }}</Label>
          <TotpCodeInput id="elevation-code" v-model="code" :disabled="pending" :invalid="!!fieldErrors.code" @complete="submit" />
          <p v-if="fieldErrors.code" class="text-sm text-destructive">{{ fieldErrors.code }}</p>
        </div>
        <Button v-if="totp" type="button" variant="link" size="sm" class="justify-self-center" @click="toggleRecovery">
          {{ recovery ? t('auth.totp.useApp') : t('auth.totp.useRecovery') }}
        </Button>
        <DialogFooter>
          <Button type="button" variant="ghost" @click="store.settle(false)">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="pending || !ready">{{ t('elevation.confirm') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
