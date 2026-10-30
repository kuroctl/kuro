<script setup lang="ts">
// Saisie d'un nom (nouveau dossier, nouveau fichier, renommage), vérifié comme le fera l'API.
// `submit` retourne un message à afficher, ou lève l'erreur de l'API (message de champ affiché
// sous la saisie, le reste en toast).
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { isValidName } from '../paths'

const props = defineProps<{
  open: boolean
  title: string
  description: string
  action: string
  initial?: string
  submit: (name: string) => Promise<string | undefined>
}>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const { t } = useI18n()
const { handleError } = useApiError()
const name = ref('')
const error = ref<string>()
const pending = ref(false)

watch(
  () => props.open,
  (open) => {
    if (open) {
      name.value = props.initial ?? ''
      error.value = undefined
    }
  },
)

async function confirm() {
  const value = name.value.trim()
  if (!isValidName(value)) {
    error.value = t('files.invalidName')
    return
  }
  pending.value = true
  try {
    error.value = await props.submit(value)
    if (!error.value) emit('update:open', false)
  } catch (caught) {
    const fields = handleError(caught, ['path', 'to'])
    error.value = fields.path ?? fields.to
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>{{ title }}</DialogTitle>
        <DialogDescription>{{ description }}</DialogDescription>
      </DialogHeader>
      <form class="grid gap-2" @submit.prevent="confirm">
        <Label for="file-name">{{ t('files.name') }}</Label>
        <Input id="file-name" v-model="name" autocomplete="off" spellcheck="false" :aria-invalid="error ? true : undefined" />
        <p v-if="error" class="text-xs text-destructive">{{ error }}</p>
        <DialogFooter class="mt-2">
          <Button type="button" variant="ghost" @click="emit('update:open', false)">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="pending || !name.trim()">{{ action }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
