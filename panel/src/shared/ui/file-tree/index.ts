// Arbre de fichiers, adapté du composant `file-tree` d'Inspira UI (licence MIT,
// https://inspira-ui.com/docs/en/components/visualization/file-tree) : icônes lucide, état
// contrôlé (`v-model:expanded`, `selected`), rôles ARIA `tree` / `treeitem`, et contenu des
// dossiers fourni par le slot (chargé à l'ouverture).
import type { HTMLAttributes, InjectionKey, Ref } from 'vue'

export type TreeItemKind = 'file' | 'folder'

export interface TreeContext {
  selected: Ref<string | undefined>
  expanded: Ref<string[]>
  indicator: boolean
  toggle: (id: string) => void
  select: (id: string, kind: TreeItemKind) => void
}

export interface TreeItemProps {
  class?: HTMLAttributes['class']
  id: string
  name: string
  isSelectable?: boolean
}

export const TREE_CONTEXT = Symbol('file-tree') as InjectionKey<TreeContext>

export { default as File } from './File.vue'
export { default as Folder } from './Folder.vue'
export { default as Tree } from './Tree.vue'
export { default as TreeIndicator } from './TreeIndicator.vue'
