// Formats partagés : le CPU de l'API (pourcent, 100 = un cœur) affiché et saisi en cœurs.
import { describe, expect, it } from 'vitest'

import { coresToPercent, formatCores, percentToCores } from '@/shared/lib/format'

describe('cores', () => {
  it('converts between the API percentage and cores, rounding the percentage', () => {
    expect(percentToCores(200)).toBe(2)
    expect(percentToCores(50)).toBe(0.5)
    expect(coresToPercent(1.5)).toBe(150)
    expect(coresToPercent(0.333)).toBe(33)
    expect(coresToPercent(percentToCores(12_800))).toBe(12_800)
  })

  it('formats cores with the plural rules of the language', () => {
    expect(formatCores(200, 'fr')).toBe('2 cœurs')
    expect(formatCores(50, 'fr')).toBe('0,5 cœur')
    expect(formatCores(150, 'fr')).toBe('1,5 cœur')
    expect(formatCores(100, 'en')).toBe('1 core')
    expect(formatCores(50, 'en')).toBe('0.5 cores')
    expect(formatCores(125, 'en')).toBe('1.25 cores')
  })
})
