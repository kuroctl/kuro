// Règles ESLint du panel, partagées avec ses extensions (ADR-0051 § 8) : une extension, rangée
// hors du panel, les reprend dans son propre `eslint.config.js` (ESLint ignore les fichiers hors
// du dossier de sa configuration) :
//
//   import { kuroPanelLint } from '<chemin>/kuro/panel/eslint.shared.js'
//   export default kuroPanelLint({ pages: ['pages/**/*.vue'] })
//
// Les plugins se résolvent depuis le panel : mêmes versions, rien à installer côté extension.
import vueI18n from '@intlify/eslint-plugin-vue-i18n'
import { defineConfigWithVueTs, vueTsConfigs } from '@vue/eslint-config-typescript'
import pluginVue from 'eslint-plugin-vue'

/**
 * @param {{ ignores?: string[], pages?: string[] }} options
 *   `ignores` : fichiers à ignorer ; `pages` : composants d'un mot permis (pages, primitives).
 */
export function kuroPanelLint({ ignores = [], pages = [] } = {}) {
  return defineConfigWithVueTs(
    { ignores },
    pluginVue.configs['flat/recommended'],
    vueTsConfigs.recommended,
    {
      rules: {
        // La mise en page relève d'un formateur, pas du linter.
        'vue/max-attributes-per-line': 'off',
        'vue/singleline-html-element-content-newline': 'off',
        // Props TypeScript optionnelles : `undefined` est une valeur voulue.
        'vue/require-default-prop': 'off',
      },
    },
    {
      files: ['**/*.vue'],
      plugins: { '@intlify/vue-i18n': vueI18n },
      rules: {
        // Tout texte affiché passe par `t()` : FR, EN, puis les langues à venir.
        '@intlify/vue-i18n/no-raw-text': 'error',
      },
    },
    ...(pages.length > 0 ? [{ files: pages, rules: { 'vue/multi-word-component-names': 'off' } }] : []),
  )
}
