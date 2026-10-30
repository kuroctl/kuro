<script setup lang="ts">
// Emplacement d'un nœud : libellé, région, lieu et position sur la carte. Sans coordonnées
// propres, le nœud est placé au centre de sa région (affiché en pointillé). Seuls les champs
// modifiés sont envoyés.
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { useRegions } from '@/features/regions/regions.queries'
import type { Schemas } from '@/shared/api/types'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { type Coordinates, MapPicker } from '@/shared/ui/map'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { UnsavedBar } from '@/shared/ui/unsaved-bar'

import { type Node, useNodesApi } from '../nodes.api'
import { useInvalidateNodes } from '../nodes.queries'

const NONE = '-'
const FIELDS = ['display_name', 'region_id', 'location', 'coordinates'] as const

const props = defineProps<{ node: Node; editable: boolean }>()
const { t } = useI18n()
const { data: regions } = useRegions()
const api = useNodesApi()
const invalidate = useInvalidateNodes()
const { handleError } = useApiError()

const form = reactive({ displayName: '', region: NONE, location: '' })
const coordinates = ref<Coordinates | null>(null)
const errors = ref<FieldErrors>({})
const pending = ref(false)

function load(node: Node) {
  form.displayName = node.display_name ?? ''
  form.region = node.region?.id ?? NONE
  form.location = node.location ?? ''
  coordinates.value = node.coordinates ?? null
}
watch(() => props.node, load, { immediate: true })

const regionCenter = computed<Coordinates | null>(() => {
  const region = regions.value?.find((r) => r.id === form.region)
  return region ? { latitude: region.latitude, longitude: region.longitude } : null
})
const regionLabel = computed(() => regions.value?.find((r) => r.id === form.region)?.name ?? t('nodes.location.noRegion'))

const text = (value: string) => (value.trim() === '' ? null : value.trim())
function same(a?: Coordinates | null, b?: Coordinates | null): boolean {
  if (!a || !b) return !a && !b
  return a.latitude === b.latitude && a.longitude === b.longitude
}

const patch = computed(() => {
  const node = props.node
  const body: Schemas['UpdateNodeRequest'] = {}
  const region = form.region === NONE ? null : form.region
  if (text(form.displayName) !== (node.display_name ?? null)) body.display_name = text(form.displayName)
  if (region !== (node.region?.id ?? null)) body.region_id = region
  if (text(form.location) !== (node.location ?? null)) body.location = text(form.location)
  if (!same(coordinates.value, node.coordinates)) body.coordinates = coordinates.value
  return body
})
const dirty = computed(() => Object.keys(patch.value).length > 0)

async function save() {
  pending.value = true
  errors.value = {}
  try {
    load(await api.update(props.node.id, patch.value))
    toast.success(t('nodes.location.saved'))
    await invalidate()
  } catch (error) {
    errors.value = handleError(error, FIELDS)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <form @submit.prevent="save">
    <Card>
      <CardHeader>
        <CardTitle>{{ t('nodes.location.title') }}</CardTitle>
        <CardDescription>{{ t('nodes.location.help') }}</CardDescription>
      </CardHeader>
      <CardContent class="grid gap-4">
        <div class="grid gap-4 sm:grid-cols-3">
          <div class="grid content-start gap-1.5">
            <Label for="node-display-name">{{ t('nodes.location.displayName') }}</Label>
            <Input
              id="node-display-name"
              v-model="form.displayName"
              :placeholder="node.name"
              :disabled="!editable"
              :aria-invalid="!!errors.display_name"
            />
            <p v-if="errors.display_name" class="text-xs text-destructive">{{ errors.display_name }}</p>
            <p v-else class="text-xs text-muted-foreground">{{ t('nodes.location.displayNameHelp', { name: node.name }) }}</p>
          </div>
          <div class="grid content-start gap-1.5">
            <Label for="node-region">{{ t('nodes.location.region') }}</Label>
            <Select v-model="form.region" :disabled="!editable">
              <SelectTrigger id="node-region" class="w-full" :aria-invalid="!!errors.region_id">
                <SelectValue>{{ regionLabel }}</SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem :value="NONE">{{ t('nodes.location.noRegion') }}</SelectItem>
                <SelectItem v-for="region in regions ?? []" :key="region.id" :value="region.id">{{ region.name }}</SelectItem>
              </SelectContent>
            </Select>
            <p v-if="errors.region_id" class="text-xs text-destructive">{{ errors.region_id }}</p>
          </div>
          <div class="grid content-start gap-1.5">
            <Label for="node-location">{{ t('nodes.location.place') }}</Label>
            <Input id="node-location" v-model="form.location" :disabled="!editable" :aria-invalid="!!errors.location" />
            <p v-if="errors.location" class="text-xs text-destructive">{{ errors.location }}</p>
            <p v-else class="text-xs text-muted-foreground">{{ t('nodes.location.placeHelp') }}</p>
          </div>
        </div>
        <div class="grid gap-1.5">
          <div class="flex items-center justify-between gap-2">
            <p class="text-sm font-medium">{{ t('nodes.location.coordinates') }}</p>
            <Button v-if="editable && coordinates && regionCenter" type="button" variant="link" @click="coordinates = null">
              {{ t('nodes.location.useRegion') }}
            </Button>
          </div>
          <MapPicker id="node-coordinates" v-model="coordinates" :fallback="regionCenter" :disabled="!editable" />
          <p v-if="errors.coordinates" class="text-xs text-destructive">{{ errors.coordinates }}</p>
        </div>
      </CardContent>
    </Card>
    <UnsavedBar v-if="editable" class="mt-4" :show="dirty" :pending="pending" @reset="load(node)" />
  </form>
</template>
