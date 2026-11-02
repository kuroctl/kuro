// Sous-menu d'une section : pages seules ou groupées, permissions, page active, recherche.
import { Boxes, Package } from '@lucide/vue'
import { describe, expect, it } from 'vitest'

import { filterEntries } from '@/shared/layout/nav'
import type { Section, SubnavItem } from '@/shared/layout/sections'
import { subnavEntries } from '@/shared/layout/subnav'

const section = (items: SubnavItem[]): Section => ({ key: 'shop', label: 'shop', icon: Boxes, to: '/shop', title: 'shop', items })
const t = (key: string) => key.toUpperCase()
const everything = () => true

const shop = section([
  { label: 'home', icon: Boxes, to: '/shop' },
  {
    label: 'catalog',
    icon: Package,
    children: [
      { label: 'products', to: '/shop/catalog' },
      { label: 'regions', to: '/shop/catalog/regions' },
      { label: 'pricing', to: '/shop/pricing', permission: 'shop.pricing' },
    ],
  },
  { label: 'customers', icon: Boxes, to: '/shop/customers' },
])

const active = (path: string, can = everything) =>
  subnavEntries(shop, path, can, t)
    .flatMap((entry) => [entry, ...(entry.children ?? [])])
    .filter((entry) => entry.active)
    .map((entry) => entry.key)

describe('subnav entries', () => {
  it('keeps a flat list of pages as it was', () => {
    const flat = section([{ label: 'home', icon: Boxes, to: '/shop', soon: false }])

    expect(subnavEntries(flat, '/shop', everything, t)).toEqual([{ key: 'home', label: 'HOME', icon: Boxes, to: '/shop', soon: false, active: true }])
  })

  it('turns a group into a collapsible entry of its pages', () => {
    const [, catalog] = subnavEntries(shop, '/shop', everything, t)

    expect(catalog).toMatchObject({ key: 'catalog', label: 'CATALOG', icon: Package })
    expect(catalog?.to).toBeUndefined()
    expect(catalog?.children?.map((child) => [child.label, child.to])).toEqual([
      ['PRODUCTS', '/shop/catalog'],
      ['REGIONS', '/shop/catalog/regions'],
      ['PRICING', '/shop/pricing'],
    ])
  })

  it('hides the pages of a group without their permission, and the group without any page', () => {
    const only = (permission: string) => (asked: string) => asked === permission
    const pricing = section([{ label: 'catalog', icon: Package, children: [{ label: 'pricing', to: '/shop/pricing', permission: 'shop.pricing' }] }])

    expect(subnavEntries(shop, '/shop', only('none'), t)[1]?.children?.map((child) => child.key)).toEqual(['products', 'regions'])
    expect(subnavEntries(pricing, '/shop', only('none'), t)).toEqual([])
    expect(subnavEntries(pricing, '/shop', only('shop.pricing'), t)).toHaveLength(1)
  })

  it('hides a whole group without its own permission', () => {
    const guarded = section([{ label: 'catalog', icon: Package, permission: 'shop.catalog', children: [{ label: 'products', to: '/shop/catalog' }] }])

    expect(subnavEntries(guarded, '/shop', () => false, t)).toEqual([])
  })

  it('marks the page of a group active, with the sub-pages no other entry claims', () => {
    expect(active('/shop/catalog')).toEqual(['products'])
    expect(active('/shop/catalog/42')).toEqual(['products'])
    expect(active('/shop/catalog/regions')).toEqual(['regions'])
    expect(active('/shop/catalog/regions/eu')).toEqual(['regions'])
  })

  it('leaves to the section root only the sub-pages no entry claims, groups included', () => {
    expect(active('/shop')).toEqual(['home'])
    expect(active('/shop/42')).toEqual(['home'])
    expect(active('/shop/pricing')).toEqual(['pricing'])
    expect(active('/shop/customers/7')).toEqual(['customers'])
  })

  it('still lets a page hidden by its permission claim its sub-pages', () => {
    expect(active('/shop/pricing', () => false)).toEqual([])
  })

  it('finds the pages of a group with the menu search', () => {
    const entries = subnavEntries(shop, '/shop', everything, t)

    expect(filterEntries(entries, 'regi').map((entry) => [entry.key, entry.children?.map((child) => child.key)])).toEqual([['catalog', ['regions']]])
    expect(filterEntries(entries, 'catal')[0]?.children).toHaveLength(3)
  })
})
