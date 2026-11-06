<script setup lang="ts">
// Région et offre d'une machine à louer (ADR-0065) : les offres viennent du fournisseur de la
// région, de la moins chère à la plus chère, avec leurs ressources et leur prix mensuel.
import { computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import { useApiError } from '@/shared/api/useApiError'
import { Label } from '@/shared/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Skeleton } from '@/shared/ui/skeleton'

import { useMachineOffers, useRentableRegions } from '../machines.queries'
import { formatMemory, formatMonthly } from '../status'

const region = defineModel<string>('region')
const offer = defineModel<string>('offer')
defineProps<{ errors?: Partial<Record<'region_id' | 'offer', string>> }>()

const { t, locale } = useI18n()
const { can } = usePermissions()
const { message } = useApiError()
const regions = useRentableRegions(() => can('system.settings.read'))
const { data: offers, isFetching, error } = useMachineOffers(region)

// Une seule région possible : choisie d'office.
watch(
  regions,
  (list) => {
    if (!region.value && list.length === 1) region.value = list[0]?.id
  },
  { immediate: true },
)
watch(region, () => (offer.value = undefined))

const describe = (name: string) => {
  const o = offers.value?.find((candidate) => candidate.name === name)
  if (!o) return name
  return t('machines.offer', {
    name: o.name,
    vcpus: o.vcpus,
    memory: formatMemory(o.memory_mb, locale.value),
    disk: o.disk_gb,
    price: formatMonthly(o.monthly_cost, locale.value),
  })
}
const offerLabel = computed(() => (offer.value ? describe(offer.value) : undefined))
</script>

<template>
  <div class="grid gap-4 sm:grid-cols-2">
    <div class="grid content-start gap-1.5">
      <Label for="machine-region">{{ t('machines.fields.region') }}</Label>
      <Select v-model="region">
        <SelectTrigger id="machine-region" class="w-full" :aria-invalid="!!errors?.region_id">
          <SelectValue :placeholder="t('machines.fields.regionPlaceholder')" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem v-for="r in regions" :key="r.id" :value="r.id">{{ r.name }}</SelectItem>
        </SelectContent>
      </Select>
      <p v-if="errors?.region_id" class="text-xs text-destructive">{{ errors.region_id }}</p>
      <p v-else-if="regions.length === 0" class="text-xs text-muted-foreground">{{ t('machines.fields.noRegion') }}</p>
    </div>
    <div class="grid content-start gap-1.5">
      <Label for="machine-offer">{{ t('machines.fields.offer') }}</Label>
      <Skeleton v-if="region && isFetching && !offers" class="h-9" />
      <Select v-else v-model="offer" :disabled="!region || !offers?.length">
        <SelectTrigger id="machine-offer" class="w-full" :aria-invalid="!!errors?.offer">
          <SelectValue :placeholder="t('machines.fields.offerPlaceholder')">{{ offerLabel }}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem v-for="o in offers ?? []" :key="o.name" :value="o.name">{{ describe(o.name) }}</SelectItem>
        </SelectContent>
      </Select>
      <p v-if="errors?.offer" class="text-xs text-destructive">{{ errors.offer }}</p>
      <p v-else-if="error" role="alert" class="text-xs text-destructive">{{ message(error) }}</p>
      <p v-else class="text-xs text-muted-foreground">{{ t('machines.fields.offerHelp') }}</p>
    </div>
  </div>
</template>
