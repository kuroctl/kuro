<script setup lang="ts">
// Marqueur de la carte : son contenu (emplacement par défaut) devient l'élément du marqueur,
// l'emplacement `popup` sa bulle, ouverte au clic. Un marqueur décoratif (`decorative`) laisse
// passer les clics et reste sous les autres.
import { Marker, Popup } from "maplibre-gl"
import { inject, onBeforeUnmount, onMounted, useSlots, useTemplateRef, watch } from "vue"

import { type Coordinates, MAP_KEY } from "./context"

const props = withDefaults(defineProps<{ position: Coordinates, draggable?: boolean, decorative?: boolean }>(), {
  draggable: false,
  decorative: false,
})
const emit = defineEmits<{ moved: [point: Coordinates] }>()
const slots = useSlots()

const map = inject(MAP_KEY)
const element = useTemplateRef<HTMLDivElement>("element")
const bubble = useTemplateRef<HTMLDivElement>("bubble")
let marker: Marker | undefined

onMounted(() => {
  if (!map?.value || !element.value) return
  marker = new Marker({ element: element.value, draggable: props.draggable })
    .setLngLat([props.position.longitude, props.position.latitude])
    .addTo(map.value)
  if (slots.popup && bubble.value) {
    marker.setPopup(new Popup({ offset: 14, closeButton: false, maxWidth: "280px" }).setDOMContent(bubble.value))
  }
  marker.on("dragend", () => {
    const point = marker?.getLngLat()
    if (point) emit("moved", { latitude: point.lat, longitude: point.lng })
  })
})

watch(
  () => [props.position.latitude, props.position.longitude],
  () => marker?.setLngLat([props.position.longitude, props.position.latitude]),
)

onBeforeUnmount(() => marker?.remove())
</script>

<template>
  <div>
    <div ref="element" data-slot="map-marker" :class="decorative ? 'pointer-events-none z-0' : 'z-[1] cursor-pointer'">
      <slot />
    </div>
    <div v-if="$slots.popup" ref="bubble" data-slot="map-popup" class="grid gap-1 text-sm">
      <slot name="popup" />
    </div>
  </div>
</template>
