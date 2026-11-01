// ESLint (flat config) : Vue + TypeScript, et aucun texte en dur dans les templates (vue-i18n).
// Les règles sont dans `eslint.shared.js`, que reprennent aussi les extensions du panel.
import { kuroPanelLint } from './eslint.shared.js'

export default kuroPanelLint({
  ignores: ['dist/**', 'coverage/**', 'src/shared/api/schema.d.ts', 'src/typed-router.d.ts'],
  // Pages du routage par fichiers (`login.vue`, `index.vue`) et composants shadcn d'un mot.
  pages: ['src/pages/**/*.vue', 'src/shared/ui/**/*.vue', 'src/App.vue', 'tests/fixtures/*/pages/**/*.vue'],
})
