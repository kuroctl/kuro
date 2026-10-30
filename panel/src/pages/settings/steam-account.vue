<script setup lang="ts">
// Compte Steam de l'installation (ADR-0033) : pour les serveurs que SteamCMD refuse en anonyme
// (Arma 3, DayZ). Le mot de passe est chiffré par l'API et jamais réaffiché : le champ reste vide
// une fois le compte en place.
import { KeyRound, Trash2 } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { useSteamApi } from '@/features/steam/steam.api'
import { useInvalidateSteamAccount, useSteamAccount } from '@/features/steam/steam.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Skeleton } from '@/shared/ui/skeleton'

const { t, locale } = useI18n()
const { can } = usePermissions()
const allowed = computed(() => can('system.steam_accounts.manage'))
const { data: account, error } = useSteamAccount(allowed)
const api = useSteamApi()
const invalidate = useInvalidateSteamAccount()
const { handleError, message } = useApiError()

const since = computed(() =>
  account.value?.updated_at
    ? new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(account.value.updated_at))
    : undefined,
)
const username = ref('')
const password = ref('')
const errors = ref<FieldErrors>({})
const pending = ref<'save' | 'remove'>()
const confirmRemove = ref(false)
// L'identifiant enregistré pré-remplit le champ : il n'est pas secret.
watch(
  () => account.value?.username,
  (saved) => {
    if (saved && !username.value) username.value = saved
  },
  { immediate: true },
)

async function save() {
  pending.value = 'save'
  errors.value = {}
  try {
    username.value = username.value.trim()
    await api.set(username.value, password.value)
    password.value = ''
    toast.success(t('steamAccount.saved'))
    await invalidate()
  } catch (failure) {
    errors.value = handleError(failure, ['username', 'password'])
  } finally {
    pending.value = undefined
  }
}

async function remove() {
  pending.value = 'remove'
  try {
    await api.remove()
    confirmRemove.value = false
    username.value = ''
    toast.success(t('steamAccount.removed'))
    await invalidate()
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = undefined
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.settings.steam')" />
    <div class="grid max-w-4xl min-w-0 gap-6 p-6">
      <p v-if="!allowed" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('steamAccount.forbidden') }}
      </p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <Skeleton v-else-if="!account" class="h-60" />
      <Card v-else>
        <CardHeader>
          <CardTitle class="flex items-center gap-2">
            {{ t('steamAccount.title') }}
            <Badge v-if="account.configured" variant="success">{{ t('steamAccount.configured') }}</Badge>
            <Badge v-else variant="secondary">{{ t('steamAccount.none') }}</Badge>
          </CardTitle>
          <CardDescription>{{ t('steamAccount.help') }}</CardDescription>
        </CardHeader>
        <CardContent>
          <form class="grid gap-4" @submit.prevent="save">
            <ul class="grid list-disc gap-1 pl-5 text-sm text-muted-foreground">
              <li>{{ t('steamAccount.dedicated') }}</li>
              <li>{{ t('steamAccount.guard') }}</li>
              <li>{{ t('steamAccount.ownership') }}</li>
            </ul>
            <p v-if="since" class="text-sm text-muted-foreground">{{ t('steamAccount.since', { date: since }) }}</p>
            <div class="grid gap-1.5">
              <Label for="steam-username">{{ t('steamAccount.username') }}</Label>
              <Input
                id="steam-username"
                v-model="username"
                autocomplete="off"
                spellcheck="false"
                :aria-invalid="!!errors.username"
              />
              <p v-if="errors.username" class="text-sm text-destructive">{{ errors.username }}</p>
            </div>
            <div class="grid gap-1.5">
              <Label for="steam-password">{{ account.configured ? t('steamAccount.replacePassword') : t('steamAccount.password') }}</Label>
              <Input
                id="steam-password"
                v-model="password"
                type="password"
                autocomplete="new-password"
                spellcheck="false"
                :aria-invalid="!!errors.password"
              />
              <p v-if="errors.password" class="text-sm text-destructive">{{ errors.password }}</p>
            </div>
            <div class="flex flex-wrap justify-end gap-2">
              <Button v-if="account.configured" type="button" variant="outline" :disabled="!!pending" @click="confirmRemove = true">
                <Trash2 />{{ t('steamAccount.remove') }}
              </Button>
              <Button type="submit" :disabled="!!pending || !username.trim() || !password">
                <KeyRound />{{ t('steamAccount.save') }}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>

    <AlertDialog v-model:open="confirmRemove">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('steamAccount.removeTitle') }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('steamAccount.removeHelp') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="!!pending" @click="remove">{{ t('steamAccount.remove') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
