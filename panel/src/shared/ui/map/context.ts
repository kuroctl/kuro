// Carte courante, fournie par `MapView` à ses marqueurs.
import type { Map as MapLibreMap } from 'maplibre-gl'
import type { InjectionKey, ShallowRef } from 'vue'

export const MAP_KEY: InjectionKey<ShallowRef<MapLibreMap | undefined>> = Symbol('map')

/** Point sur la carte (WGS 84). */
export interface Coordinates {
  latitude: number
  longitude: number
}

/** Styles OpenFreeMap selon le thème (ADR-0021). */
export const MAP_STYLES = {
  light: 'https://tiles.openfreemap.org/styles/positron',
  dark: 'https://tiles.openfreemap.org/styles/dark',
} as const
