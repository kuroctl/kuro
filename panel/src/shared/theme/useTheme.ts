// Thème : sombre par défaut, mémorisé par navigateur.
import { useColorMode } from '@vueuse/core'

export function useTheme() {
  return useColorMode({
    initialValue: 'dark',
    storageKey: 'kuro.theme',
    selector: 'html',
    attribute: 'class',
  })
}
