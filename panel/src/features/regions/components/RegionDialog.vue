<script setup lang="ts">
// Création ou modification d'une région : nom, identifiant (fixé à la création) et centre placé
// sur la carte.
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { type Coordinates, MapPicker } from '@/shared/ui/map'

import { type Region, useRegionsApi } from '../regions.api'
import { useInvalidateRegions } from '../regions.queries'

const FIELDS = ['name', 'slug', 'latitude', 'longitude'] as const

const props = defineProps<{ region?: Region }>()
const open = defineModel<boolean>('open', { default: false })
const { t } = useI18n()
const api = useRegionsApi()
const invalidate = useInvalidateRegions()
const { handleError } = useApiError()

const form = reactive({ name: '', slug: '' })
const center = ref<Coordinates | null>(null)
const errors = ref<FieldErrors>({})
const pending = ref(false)

watch(open, (opened) => {
  if (!opened) return
  form.name = props.region?.name ?? ''
  form.slug = props.region?.slug ?? ''
  center.value = props.region ? { latitude: props.region.latitude, longitude: props.region.longitude } : null
  errors.value = {}
})

const ready = computed(() => form.name.trim() !== '' && form.slug.trim() !== '' && center.value !== null)

async function submit() {
  if (!center.value) return
  pending.value = true
  errors.value = {}
  try {
    const name = form.name.trim()
    if (props.region) {
      await api.update(props.region.id, { name, ...center.value })
      toast.success(t('regions.update.done', { name }))
    } else {
      await api.create({ name, slug: form.slug.trim(), ...center.value })
      toast.success(t('regions.create.done', { name }))
    }
    await invalidate()
    open.value = false
  } catch (error) {
    errors.value = handleError(error, FIELDS)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-xl">
      <DialogHeader>
        <DialogTitle>{{ region ? t('regions.update.title', { name: region.name }) : t('regions.create.title') }}</DialogTitle>
        <DialogDescription>{{ t('regions.help') }}</DialogDescription>
      </DialogHeader>
      <form id="region-form" class="grid gap-4" @submit.prevent="submit">
        <div class="grid gap-4 sm:grid-cols-2">
          <div class="grid content-start gap-1.5">
            <Label for="region-name">{{ t('regions.fields.name') }}</Label>
            <Input id="region-name" v-model="form.name" :aria-invalid="!!errors.name" />
            <p v-if="errors.name" class="text-xs text-destructive">{{ errors.name }}</p>
          </div>
          <div class="grid content-start gap-1.5">
            <Label for="region-slug">{{ t('regions.fields.slug') }}</Label>
            <Input id="region-slug" v-model="form.slug" class="font-mono" :disabled="!!region" :aria-invalid="!!errors.slug" />
            <p v-if="errors.slug" class="text-xs text-destructive">{{ errors.slug }}</p>
            <p v-else-if="!region" class="text-xs text-muted-foreground">{{ t('regions.slugHelp') }}</p>
          </div>
        </div>
        <div class="grid gap-1.5">
          <p class="text-sm font-medium">{{ t('regions.fields.center') }}</p>
          <MapPicker id="region-center" v-model="center" />
          <p v-if="errors.latitude || errors.longitude" class="text-xs text-destructive">{{ errors.latitude ?? errors.longitude }}</p>
        </div>
      </form>
      <DialogFooter>
        <Button variant="ghost" @click="open = false">{{ t('common.cancel') }}</Button>
        <Button type="submit" form="region-form" :disabled="pending || !ready">
          {{ region ? t('common.save') : t('regions.create.submit') }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
