<script setup lang="ts">
// Fiche d'un admin (ADR-0007) : profil, rôles, identifiants (mot de passe, TOTP, sessions),
// désactivation et suppression. Chaque action n'apparaît qu'avec sa permission ; l'API applique
// l'anti-escalade (pas ses propres rôles, pas le dernier super-admin).
import { KeyRound, LogOut, Send, ShieldOff, Trash2, UserCheck, UserX } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import AdminRolesPicker from '@/features/users/components/AdminRolesPicker.vue'
import PasswordField from '@/features/users/components/PasswordField.vue'
import { type AdminUser, useUsersApi } from '@/features/users/users.api'
import { useAdminUser, useInvalidateUsers } from '@/features/users/users.queries'
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
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Skeleton } from '@/shared/ui/skeleton'

type Confirm = 'disable' | 'enable' | 'delete' | 'totp' | 'sessions'

const { t, locale } = useI18n()
const route = useRoute('/users/[id]')
const router = useRouter()
const id = computed(() => route.params.id)
const { data: user, error } = useAdminUser(id)
const { can, profile } = usePermissions()
const api = useUsersApi()
const { handleError, message } = useApiError()
const invalidate = useInvalidateUsers()

const isSelf = computed(() => user.value?.id === profile.value?.user.id)
const form = reactive({ display_name: '', email: '' })
const roles = ref<string[]>([])
const errors = ref<FieldErrors>({})
const pending = ref(false)
const confirm = ref<Confirm | null>(null)
const resetting = ref(false)
const newPassword = ref('')
const passwordError = ref<string>()

function load(value: AdminUser) {
  form.display_name = value.display_name
  form.email = value.email
  roles.value = value.roles.map((role) => role.id)
}
watch(user, (value) => value && load(value), { immediate: true })

const profileDirty = computed(() => !!user.value && (form.display_name !== user.value.display_name || form.email !== user.value.email))
const rolesDirty = computed(() => {
  const current = user.value?.roles.map((role) => role.id).sort() ?? []
  return JSON.stringify([...roles.value].sort()) !== JSON.stringify(current)
})
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }))

/** Action enveloppée : chargement, toast de succès, rechargement, erreurs traduites. */
async function run(action: () => Promise<unknown>, done: string, fields: string[] = []): Promise<boolean> {
  pending.value = true
  try {
    await action()
    toast.success(t(done, { name: user.value?.display_name ?? '' }))
    await invalidate()
    return true
  } catch (failure) {
    errors.value = handleError(failure, fields)
    return false
  } finally {
    pending.value = false
  }
}

function saveProfile() {
  if (!user.value) return
  const body: Record<string, string> = {}
  if (form.display_name !== user.value.display_name) body.display_name = form.display_name.trim()
  if (form.email !== user.value.email) body.email = form.email.trim()
  errors.value = {}
  void run(() => api.update(user.value!.id, body), 'users.detail.profileSaved', ['display_name', 'email'])
}

function saveRoles() {
  if (user.value) void run(() => api.setRoles(user.value!.id, roles.value), 'users.detail.rolesSaved')
}

async function resetPassword() {
  if (!user.value) return
  passwordError.value = undefined
  pending.value = true
  try {
    await api.resetPassword(user.value.id, newPassword.value)
    toast.success(t('users.detail.passwordReset', { name: user.value.display_name }))
    resetting.value = false
  } catch (failure) {
    passwordError.value = handleError(failure, ['password']).password
  } finally {
    pending.value = false
  }
}

async function confirmed() {
  const action = confirm.value
  const target = user.value
  if (!action || !target) return
  const done = await {
    disable: () => run(() => api.update(target.id, { disabled: true }), 'users.detail.disabled'),
    enable: () => run(() => api.update(target.id, { disabled: false }), 'users.detail.enabled'),
    totp: () => run(() => api.resetTotp(target.id), 'users.detail.totpReset'),
    sessions: () => run(() => api.revokeSessions(target.id), 'users.detail.sessionsRevoked'),
    delete: () => run(() => api.remove(target.id), 'users.detail.deleted'),
  }[action]()
  confirm.value = null
  if (done && action === 'delete') await router.push('/users')
}

function openReset() {
  newPassword.value = ''
  passwordError.value = undefined
  resetting.value = true
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="user?.display_name ?? ''">
      <Badge v-if="isSelf" variant="outline" class="font-normal">{{ t('users.you') }}</Badge>
      <Badge v-if="user?.disabled" variant="destructive" class="font-normal">{{ t('users.state.disabled') }}</Badge>
      <Badge v-if="user?.invitation_pending" variant="info" class="font-normal">{{ t('users.invitationPending') }}</Badge>
      <Badge v-else-if="user?.password_change_required" variant="warning" class="font-normal">{{ t('users.passwordChangeRequired') }}</Badge>
    </PageHeader>
    <p v-if="error" role="alert" class="m-6 rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
    <div v-else-if="!user" class="grid gap-4 p-6"><Skeleton class="h-40" /></div>
    <div v-else class="grid max-w-3xl gap-6 p-6">
      <Card>
        <CardHeader>
          <CardTitle>{{ t('users.detail.profile') }}</CardTitle>
          <CardDescription>
            {{ t('users.detail.since', { date: dates.format(new Date(user.created_at)) }) }}
            <br>
            {{ user.last_login_at ? t('users.detail.lastLogin', { date: dates.format(new Date(user.last_login_at)) }) : t('users.neverLoggedIn') }}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form class="grid gap-4 sm:grid-cols-2" @submit.prevent="saveProfile">
            <div class="grid gap-1.5">
              <Label for="user-name">{{ t('users.fields.name') }}</Label>
              <Input id="user-name" v-model="form.display_name" :disabled="!can('iam.users.update')" :aria-invalid="!!errors.display_name" />
              <p v-if="errors.display_name" class="text-sm text-destructive">{{ errors.display_name }}</p>
            </div>
            <div class="grid gap-1.5">
              <Label for="user-email">{{ t('users.fields.email') }}</Label>
              <Input id="user-email" v-model="form.email" type="email" :disabled="!can('iam.users.update')" :aria-invalid="!!errors.email" />
              <p v-if="errors.email" class="text-sm text-destructive">{{ errors.email }}</p>
            </div>
            <div v-if="can('iam.users.update')" class="flex justify-end gap-2 sm:col-span-2">
              <Button type="button" variant="ghost" :disabled="!profileDirty" @click="load(user)">{{ t('common.cancel') }}</Button>
              <Button type="submit" :disabled="pending || !profileDirty">{{ t('common.save') }}</Button>
            </div>
          </form>
        </CardContent>
      </Card>

      <Card v-if="can('iam.roles.read')">
        <CardHeader>
          <CardTitle>{{ t('users.fields.roles') }}</CardTitle>
          <CardDescription>{{ isSelf ? t('users.detail.ownRoles') : t('users.detail.rolesHelp') }}</CardDescription>
        </CardHeader>
        <CardContent class="grid gap-3">
          <AdminRolesPicker v-model="roles" :disabled="isSelf || !can('iam.roles.assign')" />
          <div v-if="can('iam.roles.assign') && !isSelf" class="flex justify-end gap-2">
            <Button variant="ghost" :disabled="!rolesDirty" @click="load(user)">{{ t('common.cancel') }}</Button>
            <Button :disabled="pending || !rolesDirty" @click="saveRoles">{{ t('common.save') }}</Button>
          </div>
        </CardContent>
      </Card>

      <Card v-if="can('iam.users.credentials')">
        <CardHeader>
          <CardTitle>{{ t('users.detail.security') }}</CardTitle>
          <CardDescription>{{ t('users.detail.securityHelp') }}</CardDescription>
        </CardHeader>
        <CardContent class="flex flex-wrap gap-2">
          <Button variant="outline" :disabled="pending" @click="openReset"><KeyRound />{{ t('users.detail.resetPassword') }}</Button>
          <Button variant="outline" :disabled="pending" @click="run(() => api.invite(user!.id), 'users.detail.invited')"><Send />{{ user.invitation_pending ? t('users.detail.reinvite') : t('users.detail.invite') }}</Button>
          <Button variant="outline" :disabled="pending || !user.totp_enabled" @click="confirm = 'totp'"><ShieldOff />{{ t('users.detail.resetTotp') }}</Button>
          <Button variant="outline" :disabled="pending" @click="confirm = 'sessions'"><LogOut />{{ t('users.detail.revokeSessions') }}</Button>
        </CardContent>
      </Card>

      <Card v-if="can('iam.users.update') || can('iam.users.delete')">
        <CardHeader>
          <CardTitle>{{ t('users.detail.account') }}</CardTitle>
          <CardDescription>{{ t('users.detail.accountHelp') }}</CardDescription>
        </CardHeader>
        <CardContent class="flex flex-wrap gap-2">
          <template v-if="can('iam.users.update')">
            <Button v-if="user.disabled" variant="secondary" :disabled="pending" @click="confirm = 'enable'"><UserCheck />{{ t('users.detail.enable') }}</Button>
            <Button v-else variant="outline" :disabled="pending || isSelf" @click="confirm = 'disable'"><UserX />{{ t('users.detail.disable') }}</Button>
          </template>
          <Button v-if="can('iam.users.delete')" variant="destructive" :disabled="pending || isSelf" @click="confirm = 'delete'"><Trash2 />{{ t('users.detail.delete') }}</Button>
        </CardContent>
      </Card>
    </div>

    <Dialog :open="resetting" @update:open="(open) => (resetting = open)">
      <DialogContent class="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{{ t('users.detail.resetPasswordTitle', { name: user?.display_name ?? '' }) }}</DialogTitle>
          <DialogDescription>{{ t('users.detail.resetPasswordHelp') }}</DialogDescription>
        </DialogHeader>
        <form class="grid gap-1.5" @submit.prevent="resetPassword">
          <Label for="reset-password">{{ t('users.fields.password') }}</Label>
          <PasswordField id="reset-password" v-model="newPassword" :invalid="!!passwordError" />
          <p v-if="passwordError" class="text-sm text-destructive">{{ passwordError }}</p>
          <DialogFooter class="mt-2">
            <Button type="button" variant="ghost" @click="resetting = false">{{ t('common.cancel') }}</Button>
            <Button type="submit" :disabled="pending || !newPassword">{{ t('users.detail.resetPassword') }}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>

    <AlertDialog :open="confirm !== null" @update:open="(open) => !open && (confirm = null)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ confirm ? t(`users.confirm.${confirm}.title`, { name: user?.display_name ?? '' }) : '' }}</AlertDialogTitle>
          <AlertDialogDescription>{{ confirm ? t(`users.confirm.${confirm}.help`) : '' }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button :variant="confirm === 'enable' ? 'default' : 'destructive'" :disabled="pending" @click="confirmed">
            {{ confirm ? t(`users.confirm.${confirm}.action`) : '' }}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
