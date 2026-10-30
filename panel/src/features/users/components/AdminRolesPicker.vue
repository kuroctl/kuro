<script setup lang="ts">
// Choix des rôles d'un admin. Un rôle qui accorde une permission qu'on ne détient pas est grisé
// (anti-escalade, ADR-0007) ; l'API reste seule juge.
import { useI18n } from 'vue-i18n'

import RoleBadge from '@/features/roles/components/RoleBadge.vue'
import { useGrantable, useRoles } from '@/features/roles/roles.queries'

const selected = defineModel<string[]>({ required: true })
defineProps<{ disabled?: boolean }>()

const { t } = useI18n()
const { data: roles } = useRoles()
const { canGrant } = useGrantable()

function toggle(id: string, checked: boolean) {
  selected.value = checked ? [...selected.value, id] : selected.value.filter((role) => role !== id)
}
</script>

<template>
  <ul class="grid gap-1.5">
    <li v-for="role in roles" :key="role.id">
      <label
        class="flex items-start gap-3 rounded-md border p-2.5 text-sm"
        :class="disabled || !canGrant(role) ? 'cursor-not-allowed opacity-60' : 'cursor-pointer hover:bg-accent/40'"
      >
        <input
          type="checkbox"
          class="mt-0.5 size-4 accent-primary"
          :checked="selected.includes(role.id)"
          :disabled="disabled || !canGrant(role)"
          :aria-label="role.name"
          @change="toggle(role.id, ($event.target as HTMLInputElement).checked)"
        >
        <span class="grid gap-1">
          <RoleBadge :name="role.name" :color="role.color" />
          <span v-if="role.description" class="text-xs text-muted-foreground">{{ role.description }}</span>
          <span v-if="!canGrant(role)" class="text-xs text-warning">{{ t('users.roles.cannotGrant') }}</span>
        </span>
      </label>
    </li>
  </ul>
</template>
