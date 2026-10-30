<script setup lang="ts">
// Création d'un rôle.
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import RoleEditor, { type RoleDraft } from '@/features/roles/components/RoleEditor.vue'
import { useRolesApi } from '@/features/roles/roles.api'
import { useInvalidateRoles } from '@/features/roles/roles.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'

const { t } = useI18n()
const router = useRouter()
const { create } = useRolesApi()
const { handleError } = useApiError()
const invalidate = useInvalidateRoles()
const pending = ref(false)
const errors = ref<FieldErrors>({})

async function save(draft: RoleDraft) {
  pending.value = true
  errors.value = {}
  try {
    const role = await create(draft)
    toast.success(t('roles.create.done', { name: role.name }))
    await invalidate()
    await router.replace(`/users/roles/${role.id}`)
  } catch (error) {
    errors.value = handleError(error, ['name', 'description', 'color'])
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('roles.create.title')" />
    <div class="max-w-3xl p-6">
      <RoleEditor :pending="pending" :errors="errors" @save="save" />
    </div>
  </div>
</template>
