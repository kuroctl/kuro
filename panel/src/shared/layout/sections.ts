// Sections du rail et leur sous-menu. Les libellés sont des clés i18n. Les extensions du panel
// (ADR-0051) ajoutent les leurs et complètent les sous-menus de Kuro (`withExtensions`).
import { Bell, Bot, Boxes, Earth, Gamepad2, Globe, LayoutDashboard, Mail, Map as MapIcon, Network, PackageSearch, ScrollText, Server, Settings, ShieldCheck, Sparkles, type LucideIcon, UserCog, UserRound, Users } from '@lucide/vue'
import extensions from 'virtual:kuro-panel-extensions'

import { withExtensions } from '@/shared/extensions'

export interface SubnavItem {
  label: string
  icon: LucideIcon
  to?: string
  /** Fonctionnalité annoncée mais pas encore disponible. */
  soon?: boolean
  /** Entrée affichée seulement avec cette permission. */
  permission?: string
}

export type BuiltinSectionKey = 'servers' | 'prefabs' | 'users' | 'settings' | 'account'

export interface Section {
  /** Section de Kuro, ou d'une extension (son nom, par convention). */
  key: BuiltinSectionKey | (string & {})
  label: string
  icon?: LucideIcon
  to: string
  title: string
  items: SubnavItem[]
  /** Section affichée dans le rail seulement avec cette permission. */
  permission?: string
}

const builtin: Section[] = [
  {
    key: 'servers',
    label: 'nav.servers',
    icon: Server,
    to: '/servers',
    title: 'subnav.servers.title',
    items: [
      { label: 'subnav.servers.overview', icon: LayoutDashboard, to: '/servers' },
      { label: 'subnav.servers.list', icon: Server, to: '/servers/list' },
      { label: 'subnav.servers.nodes', icon: Network, to: '/servers/nodes' },
      { label: 'subnav.servers.map', icon: MapIcon, to: '/servers/map' },
      { label: 'subnav.servers.regions', icon: Globe, to: '/servers/regions' },
    ],
  },
  {
    key: 'prefabs',
    label: 'nav.prefabs',
    icon: Boxes,
    to: '/prefabs',
    title: 'subnav.prefabs.title',
    items: [
      { label: 'subnav.prefabs.catalog', icon: Boxes, to: '/prefabs' },
      { label: 'subnav.prefabs.studio', icon: Sparkles, to: '/prefabs/studio' },
    ],
  },
  {
    key: 'users',
    label: 'nav.users',
    icon: Users,
    to: '/users',
    title: 'subnav.users.title',
    items: [
      { label: 'subnav.users.admins', icon: UserCog, to: '/users' },
      { label: 'subnav.users.customers', icon: Users, soon: true },
      { label: 'subnav.users.roles', icon: ShieldCheck, to: '/users/roles' },
    ],
  },
  {
    key: 'settings',
    label: 'nav.settings',
    icon: Settings,
    to: '/settings',
    title: 'subnav.settings.title',
    items: [
      { label: 'subnav.settings.project', icon: Settings, to: '/settings' },
      { label: 'subnav.settings.dns', icon: Earth, to: '/settings/dns', permission: 'system.dns.manage' },
      { label: 'subnav.settings.sources', icon: PackageSearch, to: '/settings/content-sources', permission: 'system.content_sources.manage' },
      { label: 'subnav.settings.steam', icon: Gamepad2, to: '/settings/steam-account', permission: 'system.steam_accounts.manage' },
      { label: 'subnav.settings.mail', icon: Mail, to: '/settings/mail', permission: 'system.settings.read' },
      { label: 'subnav.settings.notifications', icon: Bell, to: '/settings/notifications', permission: 'system.notifications.read' },
      { label: 'subnav.settings.audit', icon: ScrollText, to: '/settings/audit' },
    ],
  },
  {
    key: 'account',
    label: 'nav.account',
    to: '/account',
    title: 'subnav.account.title',
    items: [
      { label: 'subnav.account.profile', icon: UserRound, to: '/account' },
      { label: 'subnav.account.security', icon: ShieldCheck, to: '/account/security' },
      { label: 'subnav.account.applications', icon: Bot, to: '/account/applications' },
    ],
  },
]

export const sections: Section[] = withExtensions(builtin, extensions)

export function sectionOf(path: string): Section | undefined {
  return sections.find((section) => path === section.to || path.startsWith(`${section.to}/`))
}
