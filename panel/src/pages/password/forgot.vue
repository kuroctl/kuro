<script setup lang="ts">
// Mot de passe oublié (ADR-0049) : l'API répond de la même façon que le compte existe ou non ;
// la page dit donc seulement qu'un e-mail part si l'adresse est celle d'un compte actif.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { useAuthApi } from '@/features/auth/auth.api'
import { useSetupStatus } from '@/features/setup/setup.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import LocaleSwitch from '@/shared/layout/LocaleSwitch.vue'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

const { t } = useI18n()
const { forgotPassword } = useAuthApi()
const { handleError } = useApiError()
const { data: setup } = useSetupStatus()

const email = ref('')
const errors = ref<FieldErrors>({})
const pending = ref(false)
const sent = ref(false)

async function submit() {
  errors.value = {}
  pending.value = true
  try {
    await forgotPassword({ email: email.value })
    sent.value = true
  } catch (error) {
    errors.value = handleError(error, ['email'])
  } finally {
    pending.value = false
  }
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
      <Card class="w-full border-border/60 shadow-2xl shadow-black/40">
        <CardHeader>
          <CardTitle class="text-xl">{{ sent ? t('passwordReset.sentTitle') : t('passwordReset.forgotTitle') }}</CardTitle>
          <CardDescription>{{ sent ? t('passwordReset.sent', { email }) : t('passwordReset.forgotSubtitle') }}</CardDescription>
        </CardHeader>
        <CardContent class="flex flex-col gap-4">
          <form v-if="!sent" class="flex flex-col gap-4" @submit.prevent="submit">
            <div class="flex flex-col gap-2">
              <Label for="forgot-email">{{ t('auth.email') }}</Label>
              <Input id="forgot-email" v-model="email" type="email" autocomplete="username" required :aria-invalid="!!errors.email" />
              <p v-if="errors.email" class="text-sm text-destructive">{{ errors.email }}</p>
            </div>
            <Button type="submit" :disabled="pending">{{ t('passwordReset.send') }}</Button>
          </form>
          <p v-else class="text-xs text-muted-foreground">{{ t('passwordReset.nothingReceived') }}</p>
          <Button as-child variant="ghost">
            <RouterLink to="/login">{{ t('passwordReset.backToLogin') }}</RouterLink>
          </Button>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
