<script setup lang="ts">
// Modification et suppression d'un rôle. Un rôle système (`super-admin`) est en lecture seule.
import { Lock, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import RoleEditor, { type RoleDraft } from '@/features/roles/components/RoleEditor.vue'
import { useRolesApi } from '@/features/roles/roles.api'
import { useInvalidateRoles, useRole } from '@/features/roles/roles.queries'
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
import { Skeleton } from '@/shared/ui/skeleton'

const { t } = useI18n()
const route = useRoute('/users/roles/[id]')
const router = useRouter()
const { data: role, error } = useRole(() => route.params.id)
const { can } = usePermissions()
const api = useRolesApi()
const { handleError, message } = useApiError()
const invalidate = useInvalidateRoles()
const pending = ref(false)
const errors = ref<FieldErrors>({})
const deleting = ref(false)

const readonly = computed(() => !can('iam.roles.manage') || !!role.value?.is_system)

async function save(draft: RoleDraft) {
  const current = role.value
  if (!current) return
  const body: Record<string, unknown> = {}
  if (draft.name !== current.name) body.name = draft.name
  if (draft.description !== current.description) body.description = draft.description
  if (draft.color !== current.color) body.color = draft.color
  if (JSON.stringify([...draft.permissions].sort()) !== JSON.stringify([...current.permissions].sort())) {
    body.permissions = draft.permissions
  }
  pending.value = true
  errors.value = {}
  try {
    await api.update(current.id, body)
    toast.success(t('roles.saved', { name: draft.name }))
    await invalidate()
  } catch (failure) {
    errors.value = handleError(failure, ['name', 'description', 'color'])
  } finally {
    pending.value = false
  }
}

async function remove() {
  const current = role.value
  if (!current) return
  pending.value = true
  try {
    await api.remove(current.id)
    toast.success(t('roles.delete.done', { name: current.name }))
    await invalidate()
    await router.push('/users/roles')
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = false
    deleting.value = false
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="role?.name ?? ''">
      <Badge v-if="role?.is_system" variant="outline" class="gap-1 font-normal"><Lock />{{ t('roles.system') }}</Badge>
      <Button v-if="role && can('iam.roles.manage') && !role.is_system" size="sm" variant="destructive" @click="deleting = true"><Trash2 />{{ t('roles.delete.action') }}</Button>
    </PageHeader>
    <p v-if="error" role="alert" class="m-6 rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
    <div v-else-if="!role" class="p-6"><Skeleton class="h-80" /></div>
    <div v-else class="grid max-w-3xl gap-4 p-6">
      <p v-if="role.is_system" class="rounded-md border p-3 text-sm text-muted-foreground">{{ t('roles.systemHelp') }}</p>
      <RoleEditor :role="role" :readonly="readonly" :pending="pending" :errors="errors" @save="save" />
    </div>

    <AlertDialog :open="deleting" @update:open="(open) => (deleting = open)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('roles.delete.title', { name: role?.name ?? '' }) }}</AlertDialogTitle>
          <AlertDialogDescription>
            {{ role?.members ? t('roles.delete.withMembers', { count: role.members }) : t('roles.delete.help') }}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="pending" @click="remove">{{ t('roles.delete.confirm') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
