// Présentation d'une machine louée (ADR-0065) : ton du badge de son état, prix lisibles.
import type { BadgeVariants } from '@/shared/ui/badge'

/** Choix « nouvelle machine » à la place d'un nœud, à la création d'un serveur. */
export const NEW_MACHINE = 'new-machine'

export const machineBadge: Record<string, BadgeVariants['variant']> = {
  creating: 'warning',
  ready: 'success',
  sleeping: 'info',
  asleep: 'secondary',
  waking: 'info',
  releasing: 'warning',
}

/** Prix mensuel plafonné HT (centimes) dans la monnaie des fournisseurs connus. */
export function formatMonthly(cents: number, locale: string): string {
  return new Intl.NumberFormat(locale, { style: 'currency', currency: 'EUR' }).format(cents / 100)
}

/** Mémoire d'une offre, en Go. */
export function formatMemory(megabytes: number, locale: string): string {
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(megabytes / 1024)
}
