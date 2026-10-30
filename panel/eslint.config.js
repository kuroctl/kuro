// ESLint (flat config) : Vue + TypeScript, et aucun texte en dur dans les templates (vue-i18n).
import vueI18n from '@intlify/eslint-plugin-vue-i18n'
import { defineConfigWithVueTs, vueTsConfigs } from '@vue/eslint-config-typescript'
import pluginVue from 'eslint-plugin-vue'

export default defineConfigWithVueTs(
  {
    ignores: ['dist/**', 'coverage/**', 'src/shared/api/schema.d.ts', 'src/typed-router.d.ts'],
  },
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
  {
    // Pages du routage par fichiers (`login.vue`, `index.vue`) et composants shadcn d'un mot.
    files: ['src/pages/**/*.vue', 'src/shared/ui/**/*.vue', 'src/App.vue'],
    rules: { 'vue/multi-word-component-names': 'off' },
  },
)
