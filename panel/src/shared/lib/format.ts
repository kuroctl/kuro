// Formats localisés (unités comprises) : aucun texte en dur, Intl s'en charge ; seul le mot
// « cœur » vient des locales.

import { i18n, type Locale } from '@/shared/i18n'

const UNITS = ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte'] as const

/** Taille en octets, par paliers de 1024 (comme `docker stats`), libellée dans la langue donnée. */
export function formatBytes(bytes: number, locale: string): string {
  let value = Math.max(0, bytes)
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  return new Intl.NumberFormat(locale, {
    style: 'unit',
    unit: UNITS[unit],
    unitDisplay: 'short',
    maximumFractionDigits: value < 10 && unit > 0 ? 1 : 0,
  }).format(value)
}

export function formatPercent(percent: number, locale: string): string {
  return new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: 0 }).format(percent / 100)
}

// CPU : l'API, la base et les prefabs comptent en pourcent façon Docker (`cpu_percent`,
// 100 = un cœur entier) ; le panel n'affiche et ne saisit que des cœurs (ADR-0019).

/** Bornes et pas des champs de saisie du CPU, en cœurs (`cpu_percent` de 1 à 12 800 côté API). */
export const CORES_INPUT = {
  min: 0.01,
  max: 128,
  step: 0.5,
  formatOptions: { maximumFractionDigits: 2 } as Intl.NumberFormatOptions,
} as const

export function percentToCores(cpuPercent: number): number {
  return cpuPercent / 100
}

/** Cœurs saisis → `cpu_percent` entier attendu par l'API. */
export function coresToPercent(cores: number): number {
  return Math.round(cores * 100)
}

/** « 2 cœurs », « 0,5 cœur », « 2 cores » : nombre selon la langue, pluriel selon ses règles. */
export function formatCores(cpuPercent: number, locale: string): string {
  const cores = percentToCores(cpuPercent)
  const n = new Intl.NumberFormat(locale, CORES_INPUT.formatOptions).format(cores)
  const plural = new Intl.PluralRules(locale).select(cores)
  return i18n.global.t(`servers.cores.${plural === 'one' ? 'one' : 'other'}`, { n }, { locale: locale as Locale })
}
