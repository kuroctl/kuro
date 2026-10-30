<script setup lang="ts">
// Carte MapLibre (ADR-0021) : fond OpenFreeMap clair ou sombre selon le thème, crédit
// OpenStreetMap visible. Les marqueurs (`MapMarker`) sont placés dans son emplacement par défaut.
import "maplibre-gl/dist/maplibre-gl.css"

import { Map as MapLibreMap, NavigationControl, setWorkerUrl } from "maplibre-gl"
import type { HTMLAttributes } from "vue"
import { onBeforeUnmount, onMounted, provide, shallowRef, useTemplateRef, watch } from "vue"

import { cn } from "@/shared/lib/utils"
import { useTheme } from "@/shared/theme/useTheme"

import { type Coordinates, MAP_KEY, MAP_STYLES } from "./context"
import workerUrl from "./worker"

setWorkerUrl(workerUrl)

const props = withDefaults(defineProps<{
  center?: Coordinates
  zoom?: number
  /** Boutons de zoom. */
  controls?: boolean
  class?: HTMLAttributes["class"]
}>(), {
  center: () => ({ latitude: 30, longitude: 10 }),
  zoom: 1.4,
  controls: true,
  class: undefined,
})
const emit = defineEmits<{ click: [point: Coordinates] }>()

const container = useTemplateRef<HTMLDivElement>("container")
const map = shallowRef<MapLibreMap>()
provide(MAP_KEY, map)
const mode = useTheme()

const isDark = () => document.documentElement.classList.contains("dark")
let dark = false

onMounted(() => {
  if (!container.value) return
  dark = isDark()
  const instance = new MapLibreMap({
    container: container.value,
    style: dark ? MAP_STYLES.dark : MAP_STYLES.light,
    center: [props.center.longitude, props.center.latitude],
    zoom: props.zoom,
    attributionControl: { compact: true },
    dragRotate: false,
    pitchWithRotate: false,
  })
  if (props.controls) instance.addControl(new NavigationControl({ showCompass: false }), "top-right")
  instance.on("click", (event) => emit("click", { latitude: event.lngLat.lat, longitude: event.lngLat.lng }))
  map.value = instance
})

// Thème changé : le style suit une fois la classe posée sur le document.
watch(mode, () =>
  requestAnimationFrame(() => {
    if (!map.value || isDark() === dark) return
    dark = isDark()
    map.value.setStyle(dark ? MAP_STYLES.dark : MAP_STYLES.light)
  }),
)

onBeforeUnmount(() => map.value?.remove())

defineExpose({
  /** Cadre la carte sur ces points (un seul : centré dessus). */
  fit(points: Coordinates[], maxZoom = 5) {
    const instance = map.value
    if (!instance || points.length === 0) return
    const lng = points.map((p) => p.longitude)
    const lat = points.map((p) => p.latitude)
    instance.fitBounds([[Math.min(...lng), Math.min(...lat)], [Math.max(...lng), Math.max(...lat)]], { padding: 64, maxZoom, duration: 0 })
  },
  flyTo(point: Coordinates, zoom?: number) {
    map.value?.flyTo({ center: [point.longitude, point.latitude], zoom: zoom ?? map.value.getZoom() })
  },
})
</script>

<template>
  <div data-slot="map" :class="cn('relative isolate overflow-hidden rounded-xl border bg-muted', props.class)">
    <!-- Taille explicite : la feuille de MapLibre impose `position: relative` au conteneur. -->
    <div ref="container" class="size-full" />
    <template v-if="map">
      <slot />
    </template>
  </div>
</template>
