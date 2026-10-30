<script setup lang="ts">
// Nouveau brouillon : prefab d'exemple, ou copie d'une version du catalogue (version suivante,
// images reprises).
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { usePrefabs } from '@/features/prefabs/prefabs.queries'
import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Label } from '@/shared/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

import { useStudioApi } from '../studio.api'
import { useInvalidateStudio } from '../studio.queries'

const BLANK = 'blank'

const props = defineProps<{ open: boolean; from?: string }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const { t } = useI18n()
const router = useRouter()
const api = useStudioApi()
const invalidate = useInvalidateStudio()
const { handleError } = useApiError()
const { data: catalog } = usePrefabs(() => props.open)
const start = ref(BLANK)
const pending = ref(false)

const versions = computed(() =>
  [...(catalog.value ?? [])].sort((a, b) => a.id.localeCompare(b.id) || b.imported_at.localeCompare(a.imported_at)),
)
const label = (value: string) => (value === BLANK ? t('studio.new.blank') : t('studio.new.copyOf', { prefab: value }))

watch(
  () => props.open,
  (open) => {
    if (open) start.value = props.from ?? BLANK
  },
)

async function confirm() {
  pending.value = true
  try {
    const [id, version] = start.value === BLANK ? [] : start.value.split('@')
    const draft = await api.create(id && version ? { from: { id, version } } : {})
    await invalidate.drafts()
    emit('update:open', false)
    await router.push(`/prefabs/studio/${draft.id}`)
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>{{ t('studio.new.title') }}</DialogTitle>
        <DialogDescription>{{ t('studio.new.help') }}</DialogDescription>
      </DialogHeader>
      <form class="grid gap-2" @submit.prevent="confirm">
        <Label for="draft-start">{{ t('studio.new.start') }}</Label>
        <Select v-model="start">
          <SelectTrigger id="draft-start" class="w-full">
            <SelectValue>{{ label(start) }}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem :value="BLANK">{{ t('studio.new.blank') }}</SelectItem>
            <SelectItem v-for="entry in versions" :key="`${entry.id}@${entry.version}`" :value="`${entry.id}@${entry.version}`">
              {{ label(`${entry.id}@${entry.version}`) }}
            </SelectItem>
          </SelectContent>
        </Select>
        <DialogFooter class="mt-2">
          <Button type="button" variant="ghost" @click="emit('update:open', false)">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="pending">{{ t('studio.new.create') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
