<script setup lang="ts">
// Clés SSH du compte (ADR-0047) : elles ouvrent les sessions SFTP, sur les serveurs où le compte
// a `servers.files.sftp`. Seule l'empreinte est montrée ; retirer une clé ferme ses sessions au
// bail suivant (une minute au plus).
import { KeyRound, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { type SshKey, useAccountApi } from '@/features/account/account.api'
import { useInvalidateSshKeys, useSshKeys } from '@/features/account/account.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Skeleton } from '@/shared/ui/skeleton'
import { Textarea } from '@/shared/ui/textarea'

const { t, locale } = useI18n()
const api = useAccountApi()
const { data: keys, isPending } = useSshKeys()
const invalidate = useInvalidateSshKeys()
const { handleError } = useApiError()
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium' }))

const name = ref('')
const publicKey = ref('')
const adding = ref(false)
const fieldErrors = ref<FieldErrors>({})
const removing = ref<SshKey>()
const pendingRemoval = ref(false)

async function add() {
  adding.value = true
  fieldErrors.value = {}
  try {
    await api.addSshKey({ name: name.value, public_key: publicKey.value })
    await invalidate()
    toast.success(t('account.sshKeys.added'))
    name.value = ''
    publicKey.value = ''
  } catch (error) {
    fieldErrors.value = handleError(error, ['name', 'public_key'])
  } finally {
    adding.value = false
  }
}

async function remove() {
  const key = removing.value
  if (!key) return
  pendingRemoval.value = true
  try {
    await api.removeSshKey(key.id)
    await invalidate()
    toast.success(t('account.sshKeys.removed', { name: key.name }))
    removing.value = undefined
  } catch (error) {
    handleError(error)
  } finally {
    pendingRemoval.value = false
  }
}
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ t('account.sshKeys.title') }}</CardTitle>
      <CardDescription>{{ t('account.sshKeys.help') }}</CardDescription>
    </CardHeader>
    <CardContent class="grid gap-6 text-sm">
      <Skeleton v-if="isPending" class="h-16" />
      <p v-else-if="!keys?.length" class="text-muted-foreground">{{ t('account.sshKeys.empty') }}</p>
      <ul v-else class="grid gap-2">
        <li v-for="key in keys" :key="key.id" class="flex items-center gap-3 rounded-lg border px-3 py-2">
          <KeyRound class="size-4 shrink-0 text-muted-foreground" />
          <div class="grid min-w-0 flex-1 gap-0.5">
            <p class="font-medium">{{ key.name }}</p>
            <p class="flex min-w-0 gap-2 font-mono text-xs text-muted-foreground">
              <span>{{ key.algorithm }}</span>
              <span class="truncate">{{ key.fingerprint }}</span>
            </p>
            <p class="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
              <span>{{ t('account.sshKeys.addedOn', { date: dates.format(new Date(key.created_at)) }) }}</span>
              <span>{{
                key.last_used_at
                  ? t('account.sshKeys.usedOn', { date: dates.format(new Date(key.last_used_at)) })
                  : t('account.sshKeys.neverUsed')
              }}</span>
            </p>
          </div>
          <Button variant="ghost" size="icon" :aria-label="t('account.sshKeys.remove', { name: key.name })" @click="removing = key">
            <Trash2 class="size-4" />
          </Button>
        </li>
      </ul>

      <form class="grid gap-4" @submit.prevent="add">
        <div class="grid gap-2">
          <Label for="ssh-key-name">{{ t('account.sshKeys.name') }}</Label>
          <Input id="ssh-key-name" v-model="name" maxlength="100" required />
          <p v-if="fieldErrors.name" class="text-destructive">{{ fieldErrors.name }}</p>
        </div>
        <div class="grid gap-2">
          <Label for="ssh-key-value">{{ t('account.sshKeys.publicKey') }}</Label>
          <Textarea
            id="ssh-key-value"
            v-model="publicKey"
            class="min-h-20 font-mono text-xs"
            :placeholder="t('account.sshKeys.placeholder')"
            spellcheck="false"
            required
          />
          <p class="text-xs text-muted-foreground">{{ t('account.sshKeys.formats') }}</p>
          <p v-if="fieldErrors.public_key" class="text-destructive">{{ fieldErrors.public_key }}</p>
        </div>
        <Button type="submit" class="self-start" :disabled="adding">{{ t('account.sshKeys.add') }}</Button>
      </form>
    </CardContent>
  </Card>

  <AlertDialog :open="Boolean(removing)" @update:open="(open) => !open && (removing = undefined)">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>{{ t('account.sshKeys.confirmTitle', { name: removing?.name ?? '' }) }}</AlertDialogTitle>
        <AlertDialogDescription>{{ t('account.sshKeys.confirmHelp') }}</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
        <Button variant="destructive" :disabled="pendingRemoval" @click="remove">{{ t('account.sshKeys.confirm') }}</Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
