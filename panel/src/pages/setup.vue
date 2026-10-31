<script setup lang="ts">
// Première mise en route (ADR-0045) : jeton lu sur l'hôte, compte super admin, nom du projet,
// puis la connexion habituelle. Affichée tant que l'API n'a aucun compte admin.
import { isAxiosError } from 'axios'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { meQuery } from '@/features/account/account.queries'
import LoginFlow from '@/features/auth/components/LoginFlow.vue'
import { useSetupApi } from '@/features/setup/setup.api'
import { invalidateSetupStatus } from '@/features/setup/setup.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import LocaleSwitch from '@/shared/layout/LocaleSwitch.vue'
import { queryClient } from '@/shared/query/client'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

type Step = 'token' | 'account' | 'project' | 'login'
const STEPS: Step[] = ['token', 'account', 'project', 'login']

const { t } = useI18n()
const router = useRouter()
const { run } = useSetupApi()
const { handleError } = useApiError()

const step = ref<Step>('token')
const pending = ref(false)
const errors = ref<FieldErrors>({})

const token = ref('')
const email = ref('')
const displayName = ref('')
const password = ref('')
const confirmation = ref('')
const projectName = ref('')

const position = computed(() => STEPS.indexOf(step.value))

function goAccount() {
  errors.value = {}
  step.value = 'account'
}

function goProject() {
  errors.value = {}
  if (password.value !== confirmation.value) {
    errors.value = { confirmation: t('auth.newPassword.mismatch') }
    return
  }
  step.value = 'project'
}

async function create() {
  pending.value = true
  errors.value = {}
  try {
    await run({
      token: token.value.trim(),
      email: email.value.trim(),
      display_name: displayName.value.trim(),
      password: password.value,
      project_name: projectName.value.trim() || null,
    })
    await invalidateSetupStatus()
    step.value = 'login'
  } catch (error) {
    // Un jeton refusé n'est pas une erreur de champ pour l'API : affiché sous le jeton, à son écran.
    if (isAxiosError(error) && error.response?.data?.code === 'invalid_token') {
      errors.value = { token: t('errors.invalid_token') }
      step.value = 'token'
      return
    }
    const fields = handleError(error, ['email', 'display_name', 'password', 'project_name'])
    errors.value = fields
    // Le champ refusé est sur un écran précédent : y retourner.
    if (fields.email || fields.display_name || fields.password) step.value = 'account'
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
    <div class="relative flex w-full max-w-md flex-col gap-5">
      <ol class="flex items-center justify-center gap-2 text-xs text-muted-foreground" :aria-label="t('setup.steps')">
        <li v-for="(name, index) in STEPS" :key="name" class="flex items-center gap-2">
          <span
            class="flex size-6 items-center justify-center rounded-full border text-[11px] font-semibold"
            :class="index <= position ? 'border-primary bg-primary text-primary-foreground' : 'border-border'"
            aria-hidden="true"
          >
            {{ index + 1 }}
          </span>
          <span :class="index === position ? 'font-medium text-foreground' : ''">{{ t(`setup.stepNames.${name}`) }}</span>
          <span v-if="index < STEPS.length - 1" class="w-4 border-t border-border" aria-hidden="true" />
        </li>
      </ol>

      <LoginFlow v-if="step === 'login'" class="mx-auto" @authenticated="onAuthenticated" />

      <Card v-else class="w-full border-border/60 shadow-2xl shadow-black/40">
        <template v-if="step === 'token'">
          <CardHeader>
            <CardTitle class="text-xl">{{ t('setup.welcome.title') }}</CardTitle>
            <CardDescription>{{ t('setup.welcome.subtitle') }}</CardDescription>
          </CardHeader>
          <CardContent>
            <form class="flex flex-col gap-4" @submit.prevent="goAccount">
              <div class="flex flex-col gap-2">
                <Label for="setup-token">{{ t('setup.welcome.token') }}</Label>
                <Input id="setup-token" v-model="token" autocomplete="off" required :aria-invalid="!!errors.token" />
                <p v-if="errors.token" class="text-sm text-destructive">{{ errors.token }}</p>
                <p v-else class="text-xs text-muted-foreground">{{ t('setup.welcome.tokenHelp') }}</p>
                <pre class="rounded-md bg-muted/60 px-3 py-2 font-mono text-xs text-muted-foreground">{{ t('setup.welcome.tokenCommand') }}</pre>
              </div>
              <Button type="submit">{{ t('common.continue') }}</Button>
            </form>
          </CardContent>
        </template>

        <template v-else-if="step === 'account'">
          <CardHeader>
            <CardTitle class="text-xl">{{ t('setup.account.title') }}</CardTitle>
            <CardDescription>{{ t('setup.account.subtitle') }}</CardDescription>
          </CardHeader>
          <CardContent>
            <form class="flex flex-col gap-4" @submit.prevent="goProject">
              <div class="flex flex-col gap-2">
                <Label for="setup-name">{{ t('setup.account.name') }}</Label>
                <Input id="setup-name" v-model="displayName" autocomplete="name" required :aria-invalid="!!errors.display_name" />
                <p v-if="errors.display_name" class="text-sm text-destructive">{{ errors.display_name }}</p>
              </div>
              <div class="flex flex-col gap-2">
                <Label for="setup-email">{{ t('auth.email') }}</Label>
                <Input id="setup-email" v-model="email" type="email" autocomplete="username" required :aria-invalid="!!errors.email" />
                <p v-if="errors.email" class="text-sm text-destructive">{{ errors.email }}</p>
              </div>
              <div class="flex flex-col gap-2">
                <Label for="setup-password">{{ t('auth.password') }}</Label>
                <Input id="setup-password" v-model="password" type="password" autocomplete="new-password" required :aria-invalid="!!errors.password" />
                <p v-if="errors.password" class="text-sm text-destructive">{{ errors.password }}</p>
                <p v-else class="text-xs text-muted-foreground">{{ t('setup.account.passwordHelp') }}</p>
              </div>
              <div class="flex flex-col gap-2">
                <Label for="setup-confirmation">{{ t('auth.newPassword.confirmation') }}</Label>
                <Input id="setup-confirmation" v-model="confirmation" type="password" autocomplete="new-password" required :aria-invalid="!!errors.confirmation" />
                <p v-if="errors.confirmation" class="text-sm text-destructive">{{ errors.confirmation }}</p>
              </div>
              <Button type="submit">{{ t('common.continue') }}</Button>
              <Button type="button" variant="ghost" @click="step = 'token'">{{ t('common.back') }}</Button>
            </form>
          </CardContent>
        </template>

        <template v-else>
          <CardHeader>
            <CardTitle class="text-xl">{{ t('setup.project.title') }}</CardTitle>
            <CardDescription>{{ t('setup.project.subtitle') }}</CardDescription>
          </CardHeader>
          <CardContent>
            <form class="flex flex-col gap-4" @submit.prevent="create">
              <div class="flex flex-col gap-2">
                <Label for="setup-project">{{ t('setup.project.name') }}</Label>
                <Input id="setup-project" v-model="projectName" maxlength="80" :placeholder="t('app.name')" :aria-invalid="!!errors.project_name" />
                <p v-if="errors.project_name" class="text-sm text-destructive">{{ errors.project_name }}</p>
                <p v-else class="text-xs text-muted-foreground">{{ t('setup.project.help') }}</p>
              </div>
              <Button type="submit" :disabled="pending">{{ t('setup.project.submit') }}</Button>
              <Button type="button" variant="ghost" :disabled="pending" @click="step = 'account'">{{ t('common.back') }}</Button>
            </form>
          </CardContent>
        </template>
      </Card>
    </div>
  </div>
</template>
