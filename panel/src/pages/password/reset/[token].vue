<script setup lang="ts">
// Lien « mot de passe oublié » (ADR-0049), reçu par e-mail : choisir un nouveau mot de passe
// consomme le jeton et ferme les sessions du compte ; la suite est la connexion habituelle,
// TOTP compris.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'

import { meQuery } from '@/features/account/account.queries'
import { useAuthApi } from '@/features/auth/auth.api'
import LoginFlow from '@/features/auth/components/LoginFlow.vue'
import { useSetupStatus } from '@/features/setup/setup.queries'
import type { Schemas } from '@/shared/api/types'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import LocaleSwitch from '@/shared/layout/LocaleSwitch.vue'
import { queryClient } from '@/shared/query/client'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

/** Règle de l'API (`auth::password::check_policy`). */
const PASSWORD_LENGTH = { min: 12, max: 1024 }

const { t } = useI18n()
const route = useRoute('/password/reset/[token]')
const router = useRouter()
const { resetPassword } = useAuthApi()
const { handleError } = useApiError()
const { data: setup } = useSetupStatus()

const password = ref('')
const confirmation = ref('')
const errors = ref<FieldErrors>({})
const pending = ref(false)
/** Lien refusé (inconnu, expiré ou déjà utilisé) : on propose d'en redemander un. */
const expired = ref(false)
/** Réponse de l'API après le changement : reprise par le flux de connexion. */
const resume = ref<Schemas['LoginResponse'] | null>(null)

async function submit() {
  errors.value = {}
  const length = [...password.value].length
  if (length < PASSWORD_LENGTH.min || length > PASSWORD_LENGTH.max) {
    errors.value = { password: t('errors.weak_password') }
    return
  }
  if (password.value !== confirmation.value) {
    errors.value = { confirmation: t('auth.newPassword.mismatch') }
    return
  }
  pending.value = true
  try {
    resume.value = await resetPassword({ token: route.params.token, password: password.value })
  } catch (error) {
    errors.value = handleError(error, ['password'])
    expired.value = (error as { response?: { status?: number } }).response?.status === 403
  } finally {
    pending.value = false
  }
}

async function onAuthenticated() {
  await queryClient.prefetchQuery(meQuery)
  await router.replace('/')
}
</script>

<template>
  <div class="relative flex min-h-dvh items-center justify-center overflow-hidden bg-rail p-4">
    <div
      class="pointer-events-none absolute -top-40 left-1/2 size-[640px] -translate-x-1/2 rounded-full bg-primary/20 blur-3xl"
      aria-hidden="true"
    />
    <LocaleSwitch class="absolute top-4 right-4" />
    <div class="relative flex w-full max-w-sm flex-col items-center gap-5">
      <p v-if="setup?.project_name" class="text-lg font-bold tracking-tight">{{ setup.project_name }}</p>
      <LoginFlow v-if="resume" class="w-full" :resume="resume" @authenticated="onAuthenticated" />
      <Card v-else class="w-full border-border/60 shadow-2xl shadow-black/40">
        <CardHeader>
          <CardTitle class="text-xl">{{ t('passwordReset.resetTitle') }}</CardTitle>
          <CardDescription>{{ t('passwordReset.resetSubtitle') }}</CardDescription>
        </CardHeader>
        <CardContent class="flex flex-col gap-4">
          <form class="flex flex-col gap-4" @submit.prevent="submit">
            <div class="flex flex-col gap-2">
              <Label for="reset-password">{{ t('auth.newPassword.password') }}</Label>
              <Input id="reset-password" v-model="password" type="password" autocomplete="new-password" required :aria-invalid="!!errors.password" />
              <p v-if="errors.password" class="text-sm text-destructive">{{ errors.password }}</p>
              <p v-else class="text-xs text-muted-foreground">{{ t('setup.account.passwordHelp') }}</p>
            </div>
            <div class="flex flex-col gap-2">
              <Label for="reset-confirmation">{{ t('auth.newPassword.confirmation') }}</Label>
              <Input id="reset-confirmation" v-model="confirmation" type="password" autocomplete="new-password" required :aria-invalid="!!errors.confirmation" />
              <p v-if="errors.confirmation" class="text-sm text-destructive">{{ errors.confirmation }}</p>
            </div>
            <Button type="submit" :disabled="pending">{{ t('passwordReset.submit') }}</Button>
          </form>
          <Button v-if="expired" as-child variant="outline">
            <RouterLink to="/password/forgot">{{ t('passwordReset.newLink') }}</RouterLink>
          </Button>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
