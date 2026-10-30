// Libellés traduits du catalogue des permissions. Les clés (`iam.users.read`) contiennent des
// points : elles sont lues dans l'objet de traduction, pas comme chemin vue-i18n.
import { useI18n } from 'vue-i18n'

import type { PermissionKey } from './roles.api'

export function usePermissionText() {
  const { tm, rt } = useI18n()
  const text = (group: 'labels' | 'categories', key: string) => {
    const messages = tm(`roles.permission.${group}`) as Record<string, Parameters<typeof rt>[0]>
    const message = messages[key]
    return message === undefined ? key : rt(message)
  }
  return {
    label: (key: PermissionKey | string) => text('labels', key),
    category: (key: string) => text('categories', key),
  }
}
