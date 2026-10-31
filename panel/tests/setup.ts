// Préparation commune des tests de features : API simulée (MSW), langue fixée, nettoyage.
import { cleanup } from '@testing-library/vue'
import { afterAll, afterEach, beforeAll, beforeEach, expect, vi, type MockInstance } from 'vitest'

import { applyLocale } from '@/shared/i18n'
import { queryClient } from '@/shared/query/client'

import { server } from './support/api'
import { installLocks } from './support/locks'

// jsdom n'implémente pas `matchMedia` (thème, toasts) ni `ResizeObserver` (menus reka-ui).
window.matchMedia ??= (query: string) =>
  ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  }) as MediaQueryList
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
}
// CodeMirror mesure les positions du texte avec `Range.getClientRects`, absent de jsdom : sans
// lui, une exception dans la mesure fait perdre la complétion en cours.
Range.prototype.getClientRects ??= () => ({ length: 0, item: () => null, *[Symbol.iterator]() {} }) as unknown as DOMRectList
Range.prototype.getBoundingClientRect ??= () =>
  ({ x: 0, y: 0, width: 0, height: 0, top: 0, left: 0, right: 0, bottom: 0, toJSON: () => ({}) }) as DOMRect
// Graphiques (Unovis, d3) : jsdom n'a ni les transformations SVG ni la mesure des textes.
Object.defineProperty(SVGElement.prototype, 'transform', {
  configurable: true,
  get: () => ({ baseVal: { consolidate: () => null } }),
})
const svg = SVGElement.prototype as SVGElement & { getBBox?: () => DOMRect; getComputedTextLength?: () => number }
svg.getBBox ??= () => ({ x: 0, y: 0, width: 0, height: 0 }) as DOMRect
svg.getComputedTextLength ??= () => 0

// Menus et listes reka-ui : capture du pointeur et défilement, absents de jsdom.
Element.prototype.hasPointerCapture ??= () => false
Element.prototype.releasePointerCapture ??= () => {}
Element.prototype.setPointerCapture ??= () => {}
Element.prototype.scrollIntoView ??= () => {}
// URL locales des images (`blob:`) : jsdom ne les crée pas.
URL.createObjectURL ??= (blob: Blob) => `blob:test/${blob.size}`
URL.revokeObjectURL ??= () => {}
// Canvas absent de jsdom (couleurs du terminal) : pas de contexte, comme un navigateur sans canvas.
HTMLCanvasElement.prototype.getContext = (() => null) as typeof HTMLCanvasElement.prototype.getContext

// MapLibre a besoin de WebGL : carte simulée, et pas de worker à empaqueter.
vi.mock('maplibre-gl', () => import('./support/maplibre'))
vi.mock('@/shared/ui/map/worker', () => ({ default: 'worker.js' }))

installLocks()

// Toute requête sans handler fait échouer le test : rien ne part vers une vraie API.
beforeAll(() => server.listen({ onUnhandledFrame: 'error' }))
// Un avertissement Vue ou du routeur est un défaut (transition inopérante, route manquante…).
let warn: MockInstance<typeof console.warn>
beforeEach(() => {
  applyLocale('fr')
  warn = vi.spyOn(console, 'warn')
})
afterEach(() => {
  const warnings = warn.mock.calls.map((args) => args.map(String).join(' '))
  warn.mockRestore()
  expect(warnings).toEqual([])
  cleanup()
  server.resetHandlers()
  queryClient.clear()
})
afterAll(() => server.close())
