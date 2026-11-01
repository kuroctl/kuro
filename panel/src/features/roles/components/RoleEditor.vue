<script setup lang="ts">
// Éditeur d'un rôle : nom, description, couleur (palette ou `#rrggbb`), permissions par
// catégorie. Les permissions à venir sont grisées, comme celles qu'on ne détient pas
// (anti-escalade, ADR-0007) ; l'API reste seule juge. Les modifications s'enregistrent depuis la
// barre qui apparaît en bas dès qu'il y en a.
import { Check, ShieldAlert } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { type FieldErrors } from '@/shared/api/useApiError'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Skeleton } from '@/shared/ui/skeleton'
import { Textarea } from '@/shared/ui/textarea'
import { UnsavedBar } from '@/shared/ui/unsaved-bar'

import { isHexColor, ROLE_COLORS } from '../colors'
import { usePermissionText } from '../permissions'
import type { PermissionCategory, Role } from '../roles.api'
import { useGrantable, usePermissionCatalog } from '../roles.queries'
import RoleBadge from './RoleBadge.vue'

export interface RoleDraft {
  name: string
  description: string
  color: string
  permissions: string[]
}

const props = defineProps<{ role?: Role; readonly?: boolean; pending?: boolean; errors?: FieldErrors }>()
const emit = defineEmits<{ save: [draft: RoleDraft] }>()

const { t } = useI18n()
const { data: catalog } = usePermissionCatalog()
const { holds } = useGrantable()
const text = usePermissionText()

const draft = reactive<RoleDraft>({ name: '', description: '', color: ROLE_COLORS[0], permissions: [] })
const hex = ref<string>(ROLE_COLORS[0])

function load() {
  draft.name = props.role?.name ?? ''
  draft.description = props.role?.description ?? ''
  draft.color = props.role?.color ?? ROLE_COLORS[0]
  draft.permissions = [...(props.role?.permissions ?? [])]
  hex.value = draft.color
}
watch(() => props.role, load, { immediate: true })
watch(hex, (value) => {
  if (isHexColor(value)) draft.color = value.toLowerCase()
})

/** Permission modifiable ici : attribuable, et détenue par l'admin connecté. */
function editable(permission: PermissionCategory['permissions'][number]): boolean {
  return !props.readonly && permission.availability === 'available' && holds(permission.key)
}

function toggle(key: string, checked: boolean) {
  draft.permissions = checked ? [...new Set([...draft.permissions, key])] : draft.permissions.filter((p) => p !== key)
}

function categoryState(category: PermissionCategory): boolean | 'mixed' {
  const keys = category.permissions.filter(editable).map((p) => p.key)
  const on = keys.filter((key) => draft.permissions.includes(key)).length
  return on === 0 ? false : on === keys.length ? true : 'mixed'
}

function toggleCategory(category: PermissionCategory, checked: boolean) {
  for (const permission of category.permissions.filter(editable)) toggle(permission.key, checked)
}

const dirty = computed(() => {
  const role = props.role
  if (!role) return draft.name.trim() !== ''
  return (
    draft.name !== role.name ||
    draft.description !== role.description ||
    draft.color !== role.color ||
    JSON.stringify([...draft.permissions].sort()) !== JSON.stringify([...role.permissions].sort())
  )
})

function submit() {
  emit('save', { ...draft, name: draft.name.trim(), description: draft.description.trim() })
}
</script>

<template>
  <form class="grid gap-6" @submit.prevent="submit">
    <Card>
      <CardHeader>
        <CardTitle>{{ t('roles.editor.identity') }}</CardTitle>
        <CardDescription>{{ t('roles.editor.identityHelp') }}</CardDescription>
      </CardHeader>
      <CardContent class="grid gap-4">
        <div class="grid gap-4 sm:grid-cols-2">
          <div class="grid gap-1.5">
            <Label for="role-name">{{ t('roles.fields.name') }}</Label>
            <Input id="role-name" v-model="draft.name" maxlength="64" autocomplete="off" spellcheck="false" class="font-mono" :disabled="readonly" :aria-invalid="!!errors?.name" />
            <p v-if="errors?.name" class="text-sm text-destructive">{{ errors.name }}</p>
            <p v-else class="text-xs text-muted-foreground">{{ t('roles.editor.nameHelp') }}</p>
          </div>
          <div class="grid gap-1.5">
            <p class="text-sm font-medium">{{ t('roles.editor.preview') }}</p>
            <div class="flex h-9 items-center"><RoleBadge :name="draft.name || t('roles.editor.placeholder')" :color="draft.color" /></div>
          </div>
        </div>
        <div class="grid gap-1.5">
          <Label for="role-description">{{ t('roles.fields.description') }}</Label>
          <Textarea id="role-description" v-model="draft.description" rows="2" maxlength="256" :disabled="readonly" :aria-invalid="!!errors?.description" />
          <p v-if="errors?.description" class="text-sm text-destructive">{{ errors.description }}</p>
        </div>
        <div class="grid gap-1.5">
          <p class="text-sm font-medium">{{ t('roles.fields.color') }}</p>
          <div class="flex flex-wrap items-center gap-2" role="radiogroup" :aria-label="t('roles.fields.color')">
            <button
              v-for="color in ROLE_COLORS"
              :key="color"
              type="button"
              role="radio"
              :aria-checked="draft.color === color"
              :aria-label="color"
              :disabled="readonly"
              class="flex size-7 items-center justify-center rounded-full ring-offset-2 ring-offset-background transition disabled:cursor-not-allowed"
              :class="draft.color === color ? 'ring-2 ring-foreground' : 'hover:scale-110'"
              :style="{ backgroundColor: color }"
              @click="hex = color"
            >
              <Check v-if="draft.color === color" class="size-4 text-white" />
            </button>
            <Input v-model="hex" class="ml-1 w-28 font-mono" maxlength="7" :aria-label="t('roles.editor.hex')" :disabled="readonly" :aria-invalid="!isHexColor(hex) || !!errors?.color" />
          </div>
          <p v-if="errors?.color" class="text-sm text-destructive">{{ errors.color }}</p>
        </div>
      </CardContent>
    </Card>

    <Card>
      <CardHeader>
        <CardTitle>{{ t('roles.editor.permissions') }}</CardTitle>
        <CardDescription>{{ t('roles.editor.permissionsHelp', { count: draft.permissions.length }) }}</CardDescription>
      </CardHeader>
      <CardContent>
        <div v-if="!catalog" class="grid gap-2"><Skeleton v-for="index in 4" :key="index" class="h-16" /></div>
        <p v-else-if="role?.permissions.includes('*')" class="text-sm text-muted-foreground">{{ t('roles.editor.everything') }}</p>
        <div v-else class="grid gap-5">
          <fieldset v-for="category in catalog" :key="category.key" class="grid gap-2">
            <legend class="mb-1 flex w-full items-center gap-2 text-sm font-semibold">
              <input
                type="checkbox"
                class="size-4 accent-primary"
                :checked="categoryState(category) === true"
                :indeterminate="categoryState(category) === 'mixed'"
                :disabled="category.permissions.every((p) => !editable(p))"
                :aria-label="t('roles.editor.wholeCategory', { category: text.category(category.key) })"
                @change="toggleCategory(category, ($event.target as HTMLInputElement).checked)"
              >
              {{ text.category(category.key) }}
            </legend>
            <label
              v-for="permission in category.permissions"
              :key="permission.key"
              class="flex items-start gap-3 rounded-md border px-3 py-2 text-sm"
              :class="editable(permission) ? 'cursor-pointer hover:bg-accent/40' : 'cursor-not-allowed opacity-60'"
            >
              <input
                type="checkbox"
                class="mt-0.5 size-4 accent-primary"
                :checked="draft.permissions.includes(permission.key)"
                :disabled="!editable(permission)"
                @change="toggle(permission.key, ($event.target as HTMLInputElement).checked)"
              >
              <span class="grid gap-0.5">
                <span class="flex flex-wrap items-center gap-2">
                  {{ text.label(permission.key, permission.label) }}
                  <span v-if="permission.sensitive" class="inline-flex items-center gap-1 text-xs text-warning"><ShieldAlert class="size-3.5" />{{ t('roles.editor.sensitive') }}</span>
                  <span v-if="permission.availability === 'reserved'" class="text-xs text-muted-foreground">{{ t('common.soon') }}</span>
                  <span v-else-if="!readonly && !holds(permission.key)" class="text-xs text-muted-foreground">{{ t('roles.editor.notHeld') }}</span>
                </span>
                <span class="font-mono text-xs text-muted-foreground">{{ permission.key }}</span>
              </span>
            </label>
          </fieldset>
        </div>
      </CardContent>
    </Card>

    <UnsavedBar
      v-if="!readonly"
      :show="dirty"
      :pending="pending"
      :can-save="isHexColor(hex)"
      :save-label="role ? t('common.save') : t('roles.create.submit')"
      @reset="load"
    />
  </form>
</template>
