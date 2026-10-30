// i18n : tout texte affiché passe par vue-i18n. FR et EN d'abord ; une langue s'ajoute avec un
// dossier `src/locales/<code>/` typé `satisfies typeof fr`.
import { createI18n } from 'vue-i18n'

import en from '@/locales/en'
import fr from '@/locales/fr'
import { setRequestLanguage } from '@/shared/api/http'

export type MessageSchema = typeof fr

export const SUPPORTED_LOCALES = ['fr', 'en'] as const
export type Locale = (typeof SUPPORTED_LOCALES)[number]

const STORAGE_KEY = 'kuro.locale'

function isLocale(value: unknown): value is Locale {
  return SUPPORTED_LOCALES.includes(value as Locale)
}

/** Langue mémorisée, sinon celle du navigateur, sinon le français. */
function initialLocale(): Locale {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    if (isLocale(stored)) return stored
  } catch {
    // Stockage indisponible (navigation privée…) : on se rabat sur le navigateur.
  }
  const browser = navigator.language.slice(0, 2)
  return isLocale(browser) ? browser : 'fr'
}

// Types inférés des options : la cohérence FR/EN est garantie par `satisfies` dans les locales.
export const i18n = createI18n({
  legacy: false,
  locale: initialLocale(),
  fallbackLocale: 'en',
  messages: { fr, en },
})

export function applyLocale(locale: Locale) {
  i18n.global.locale.value = locale
  document.documentElement.lang = locale
  setRequestLanguage(locale)
  try {
    localStorage.setItem(STORAGE_KEY, locale)
  } catch {
    // Préférence non mémorisée : sans conséquence.
  }
}
