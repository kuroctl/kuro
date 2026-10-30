// Worker de MapLibre empaqueté par Vite et servi par le panel : la CSP n'accepte aucun worker
// `blob:` ni d'une autre origine (ADR-0021).
import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url'

export default workerUrl
