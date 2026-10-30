<script setup lang="ts">
// Choix d'un point : clic sur la carte ou marqueur déplacé, ou saisie de la latitude et de la
// longitude (accessible au clavier). `fallback` : point affiché en pointillé quand aucun n'est
// choisi (le centre de la région d'un nœud, par exemple).
import { computed, useTemplateRef, watch } from "vue"
import { useI18n } from "vue-i18n"

import { Label } from "@/shared/ui/label"
import { NumberField, NumberFieldContent, NumberFieldInput } from "@/shared/ui/number-field"

import type { Coordinates } from "./context"
import MapMarker from "./MapMarker.vue"
import MapView from "./MapView.vue"

const props = defineProps<{ id: string, fallback?: Coordinates | null, disabled?: boolean }>()
const model = defineModel<Coordinates | null>({ default: null })
const { t, locale } = useI18n()

const shown = computed(() => model.value ?? props.fallback ?? null)
const view = useTemplateRef<InstanceType<typeof MapView>>("view")

// Point connu après coup (région chargée ensuite) : la carte va le chercher.
watch(shown, (point, previous) => {
  if (point && !previous) view.value?.flyTo(point, 4)
})
const FORMAT = { maximumFractionDigits: 5 }

function round(value: number): number {
  return Math.round(value * 1e5) / 1e5
}

function place(point: Coordinates) {
  if (props.disabled) return
  model.value = { latitude: round(point.latitude), longitude: round(point.longitude) }
}

function setAxis(axis: keyof Coordinates, value: number | undefined) {
  if (value == null || Number.isNaN(value)) return
  place({ latitude: model.value?.latitude ?? shown.value?.latitude ?? 0, longitude: model.value?.longitude ?? shown.value?.longitude ?? 0, [axis]: value })
}
</script>

<template>
  <div class="grid gap-3">
    <MapView
      ref="view"
      class="h-64"
      :center="shown ?? undefined"
      :zoom="shown ? 3 : 1"
      @click="place"
    >
      <MapMarker v-if="shown" :position="shown" :draggable="!disabled && !!model" @moved="place">
        <span
          class="block size-4 rounded-full border-2 border-white shadow-[0_0_0_4px_color-mix(in_oklch,var(--primary)_35%,transparent)]"
          :class="model ? 'bg-primary' : 'border-dashed bg-muted-foreground/60'"
        />
      </MapMarker>
    </MapView>
    <p v-if="!disabled" class="text-xs text-muted-foreground">{{ t('map.pickHint') }}</p>
    <div class="grid gap-3 sm:grid-cols-2">
      <div class="grid gap-1.5">
        <Label :for="`${id}-latitude`">{{ t('map.latitude') }}</Label>
        <NumberField
          :model-value="model?.latitude"
          :min="-90"
          :max="90"
          :step="0.0001"
          :step-snapping="false"
          :format-options="FORMAT"
          :locale="locale"
          :disabled="disabled"
          @update:model-value="setAxis('latitude', $event)"
        >
          <NumberFieldContent><NumberFieldInput :id="`${id}-latitude`" /></NumberFieldContent>
        </NumberField>
      </div>
      <div class="grid gap-1.5">
        <Label :for="`${id}-longitude`">{{ t('map.longitude') }}</Label>
        <NumberField
          :model-value="model?.longitude"
          :min="-180"
          :max="180"
          :step="0.0001"
          :step-snapping="false"
          :format-options="FORMAT"
          :locale="locale"
          :disabled="disabled"
          @update:model-value="setAxis('longitude', $event)"
        >
          <NumberFieldContent><NumberFieldInput :id="`${id}-longitude`" /></NumberFieldContent>
        </NumberField>
      </div>
    </div>
  </div>
</template>
