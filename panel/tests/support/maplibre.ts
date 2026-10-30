// Carte simulée : MapLibre a besoin de WebGL, absent de jsdom. Les marqueurs et leurs bulles
// restent dans le DOM pour que les tests les lisent ; `click` simule un clic sur la carte.
type Handler = (event: { lngLat: { lat: number; lng: number } }) => void

export class Map {
  static last: Map | undefined
  private handlers: Record<string, Handler[]> = {}
  container: HTMLElement
  center: [number, number]
  zoom: number

  constructor(options: { container: HTMLElement; center: [number, number]; zoom: number }) {
    this.container = options.container
    this.center = options.center
    this.zoom = options.zoom
    Map.last = this
  }

  on(type: string, handler: Handler) {
    ;(this.handlers[type] ??= []).push(handler)
    return this
  }

  /** Clic simulé à ces coordonnées. */
  click(lat: number, lng: number) {
    for (const handler of this.handlers.click ?? []) handler({ lngLat: { lat, lng } })
  }

  addControl() {
    return this
  }
  setStyle() {}
  fitBounds() {}
  flyTo() {}
  getZoom() {
    return this.zoom
  }
  remove() {}
}

export class Marker {
  private element: HTMLElement
  private position = { lng: 0, lat: 0 }
  private popup: Popup | undefined

  constructor(options: { element: HTMLElement }) {
    this.element = options.element
  }
  setLngLat([lng, lat]: [number, number]) {
    this.position = { lng, lat }
    this.element.dataset.position = `${lat},${lng}`
    return this
  }
  getLngLat() {
    return this.position
  }
  setPopup(popup: Popup) {
    this.popup = popup
    return this
  }
  addTo(map: Map) {
    map.container.append(this.element)
    if (this.popup?.content) map.container.append(this.popup.content)
    return this
  }
  on() {
    return this
  }
  remove() {
    this.element.remove()
    this.popup?.content?.remove()
  }
}

export class Popup {
  content: HTMLElement | undefined
  setDOMContent(content: HTMLElement) {
    this.content = content
    return this
  }
}

export class NavigationControl {}

export function setWorkerUrl() {}
