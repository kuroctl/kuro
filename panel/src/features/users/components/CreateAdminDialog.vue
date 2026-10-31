<script setup lang="ts">
// Création d'un admin : nom, e-mail, rôles, et soit une invitation par e-mail (SMTP réglé,
// ADR-0046), soit un mot de passe initial à lui transmettre.
import { Plus } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { useMailSettings } from '@/features/mail/mail.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Switch } from '@/shared/ui/switch'

import { useUsersApi } from '../users.api'
import { useInvalidateUsers } from '../users.queries'
import AdminRolesPicker from './AdminRolesPicker.vue'
import PasswordField from './PasswordField.vue'

const { t } = useI18n()
const router = useRouter()
const api = useUsersApi()
const { can } = usePermissions()
const { handleError } = useApiError()
const invalidate = useInvalidateUsers()

const open = ref(false)
const pending = ref(false)
const errors = ref<FieldErrors>({})
const form = reactive({ display_name: '', email: '', password: '', roles: [] as string[], invite: false })
const canAssign = computed(() => can('iam.roles.assign'))
const { data: mail } = useMailSettings(() => can('system.settings.read'))
const mailConfigured = computed(() => mail.value?.configured === true)
// Invitation par défaut dès qu'un SMTP est réglé ; sinon, le mot de passe temporaire.
watch(mailConfigured, (configured) => (form.invite = configured), { immediate: true })

function reset(isOpen: boolean) {
  open.value = isOpen
  if (!isOpen) {
    Object.assign(form, { display_name: '', email: '', password: '', roles: [], invite: mailConfigured.value })
    errors.value = {}
  }
}

async function submit() {
  pending.value = true
  errors.value = {}
  try {
    const user = await api.create({
      display_name: form.display_name.trim(),
      email: form.email.trim(),
      ...(form.invite ? { invite: true } : { password: form.password }),
    })
    if (form.roles.length > 0) {
      try {
        await api.setRoles(user.id, form.roles)
      } catch (error) {
        // Le compte existe : les rôles se corrigent sur sa fiche.
        handleError(error)
      }
    }
    toast.success(t(form.invite ? 'users.create.invited' : 'users.create.done', { name: user.display_name }))
    await invalidate()
    reset(false)
    await router.push(`/users/${user.id}`)
  } catch (error) {
    errors.value = handleError(error, ['display_name', 'email', 'password'])
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="reset">
    <DialogTrigger as-child>
      <Button size="sm"><Plus />{{ t('users.create.action') }}</Button>
    </DialogTrigger>
    <DialogContent class="max-h-[90dvh] overflow-y-auto sm:max-w-lg">
      <DialogHeader>
        <DialogTitle>{{ t('users.create.title') }}</DialogTitle>
        <DialogDescription>{{ t('users.create.help') }}</DialogDescription>
      </DialogHeader>
      <form class="grid gap-4" @submit.prevent="submit">
        <div class="grid gap-1.5">
          <Label for="admin-name">{{ t('users.fields.name') }}</Label>
          <Input id="admin-name" v-model="form.display_name" autocomplete="off" :aria-invalid="!!errors.display_name" />
          <p v-if="errors.display_name" class="text-sm text-destructive">{{ errors.display_name }}</p>
        </div>
        <div class="grid gap-1.5">
          <Label for="admin-email">{{ t('users.fields.email') }}</Label>
          <Input id="admin-email" v-model="form.email" type="email" autocomplete="off" :aria-invalid="!!errors.email" />
          <p v-if="errors.email" class="text-sm text-destructive">{{ errors.email }}</p>
        </div>
        <div v-if="mailConfigured" class="flex items-center justify-between gap-4 rounded-lg border px-3 py-2">
          <div class="grid gap-0.5">
            <Label for="admin-invite">{{ t('users.create.invite') }}</Label>
            <p class="text-xs text-muted-foreground">{{ t('users.create.inviteHelp') }}</p>
          </div>
          <Switch id="admin-invite" v-model="form.invite" />
        </div>
        <p v-else class="text-xs text-muted-foreground">{{ t('users.create.noMail') }}</p>
        <div v-if="!form.invite" class="grid gap-1.5">
          <Label for="admin-password">{{ t('users.fields.password') }}</Label>
          <PasswordField id="admin-password" v-model="form.password" :invalid="!!errors.password" />
          <p v-if="errors.password" class="text-sm text-destructive">{{ errors.password }}</p>
          <p v-else class="text-xs text-muted-foreground">{{ t('users.password.help') }}</p>
        </div>
        <div v-if="canAssign" class="grid gap-1.5">
          <p class="text-sm font-medium">{{ t('users.fields.roles') }}</p>
          <AdminRolesPicker v-model="form.roles" />
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" @click="reset(false)">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="pending">{{ form.invite ? t('users.create.submitInvite') : t('users.create.submit') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
