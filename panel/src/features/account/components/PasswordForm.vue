<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { useApiError, type FieldErrors } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { useAccountApi } from '../account.api'

const { t } = useI18n()
const { changePassword } = useAccountApi()
const { handleError } = useApiError()

const currentPassword = ref('')
const newPassword = ref('')
const pending = ref(false)
const fieldErrors = ref<FieldErrors>({})

async function submit() {
  pending.value = true
  fieldErrors.value = {}
  try {
    await changePassword({ current_password: currentPassword.value, new_password: newPassword.value })
    toast.success(t('account.security.passwordChanged'))
    currentPassword.value = ''
    newPassword.value = ''
  } catch (error) {
    fieldErrors.value = handleError(error, ['current_password', 'new_password'])
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <form class="flex max-w-sm flex-col gap-4" @submit.prevent="submit">
    <div class="flex flex-col gap-2">
      <Label for="current-password">{{ t('account.security.currentPassword') }}</Label>
      <Input id="current-password" v-model="currentPassword" type="password" autocomplete="current-password" required />
      <p v-if="fieldErrors.current_password" class="text-sm text-destructive">{{ fieldErrors.current_password }}</p>
    </div>
    <div class="flex flex-col gap-2">
      <Label for="new-password">{{ t('account.security.newPassword') }}</Label>
      <Input id="new-password" v-model="newPassword" type="password" autocomplete="new-password" required />
      <p v-if="fieldErrors.new_password" class="text-sm text-destructive">{{ fieldErrors.new_password }}</p>
    </div>
    <Button type="submit" class="self-start" :disabled="pending">{{ t('common.save') }}</Button>
  </form>
</template>
