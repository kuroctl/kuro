// Module produit par le plugin `kuroPanelExtensions` de `vite.config.ts` : les extensions
// déclarées par `KURO_PANEL_EXTENSIONS`, liste vide sans elles.
declare module 'virtual:kuro-panel-extensions' {
  import type { PanelExtension } from '@/shared/extensions'

  const extensions: PanelExtension[]
  export default extensions
}
