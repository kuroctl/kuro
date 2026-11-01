// Extensions du panel (ADR-0051 § 8) : fusion de la navigation et des traductions, lecture de
// `KURO_PANEL_EXTENSIONS`.
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { Boxes } from '@lucide/vue'
import { describe, expect, it } from 'vitest'

import { isBare, kuroPanelExtensions, parseExtensions } from '../../build/extensions'
import { extensionMessages, type PanelExtension, withExtensions } from '@/shared/extensions'
import type { Section } from '@/shared/layout/sections'

const section = (key: string): Section => ({ key, label: key, icon: Boxes, to: `/${key}`, title: key, items: [] })
const builtin = [section('servers'), section('users'), section('settings'), section('account')]

describe('navigation of the extensions', () => {
  it('puts extension sections before the settings and extends Kuro sections', () => {
    const shop: PanelExtension = {
      name: 'shop',
      sections: [section('shop')],
      items: { users: [{ label: 'ext.shop.customers', icon: Boxes, to: '/shop/customers' }] },
    }

    const merged = withExtensions(builtin, [shop])

    expect(merged.map((s) => s.key)).toEqual(['servers', 'users', 'shop', 'settings', 'account'])
    expect(merged.find((s) => s.key === 'users')?.items.map((i) => i.to)).toEqual(['/shop/customers'])
  })

  it('leaves the navigation untouched without extension', () => {
    expect(withExtensions(builtin, [])).toEqual(builtin)
  })

  it('mounts the translations under the name of each extension', () => {
    const extensions: PanelExtension[] = [
      { name: 'shop', messages: { fr: { title: 'Boutique' }, en: { title: 'Shop' } } },
      { name: 'bare' },
    ]

    expect(extensionMessages(extensions, 'en')).toEqual({ shop: { title: 'Shop' }, bare: {} })
  })
})

describe('KURO_PANEL_EXTENSIONS', () => {
  const root = mkdtempSync(join(tmpdir(), 'kuro-panel-ext-'))
  writeFileSync(join(root, 'extension.ts'), 'export default {}\n')

  it('reads <name>=<dir> entries', () => {
    expect(parseExtensions(` shop=${root} `, '/')).toEqual([{ name: 'shop', dir: root }])
    expect(parseExtensions(undefined, '/')).toEqual([])
  })

  it('refuses malformed, duplicated or missing extensions', () => {
    expect(() => parseExtensions(root, '/')).toThrow(/invalid entry/)
    expect(() => parseExtensions(`Shop=${root}`, '/')).toThrow(/invalid entry/)
    expect(() => parseExtensions(`shop=${root},shop=${root}`, '/')).toThrow(/listed twice/)
    expect(() => parseExtensions(`shop=${join(root, 'missing')}`, '/')).toThrow(/no extension.ts/)
  })
})

describe('tailwind sources', () => {
  it('declares the extension folders to Tailwind in the main stylesheet only', () => {
    type Transform = (code: string, id: string) => string | undefined
    const handler = (plugin: ReturnType<typeof kuroPanelExtensions>) => (plugin.transform as { handler: Transform }).handler
    const transform = handler(kuroPanelExtensions([{ name: 'shop', dir: '/ext/shop' }], '/kuro/panel'))

    expect(transform('@import "tailwindcss";', '/kuro/panel/src/assets/main.css')).toContain('@source "/ext/shop";')
    expect(transform('a{}', '/kuro/panel/src/other.css')).toBeUndefined()
    expect(handler(kuroPanelExtensions([], '/p'))('x', '/p/src/assets/main.css')).toBeUndefined()
  })
})

describe('bare imports of an extension', () => {
  it('recognizes package imports only', () => {
    for (const id of ['vue', '@lucide/vue', 'vue-i18n/dist/x.js']) expect(isBare(id), id).toBe(true)
    for (const id of ['./x', '../x', '/abs', 'C:/x', '@/shared/ui', 'virtual:kuro-panel-extensions', '\0x']) {
      expect(isBare(id), id).toBe(false)
    }
  })
})
