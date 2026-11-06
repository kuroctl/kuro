<script setup lang="ts">
// Palette de commandes (Ctrl+K ou ⌘K) : aller à une page, un serveur ou un nœud, ou lancer une
// création. Sans recherche, elle propose les derniers serveurs et nœuds ouverts.
import { type LucideIcon, Network, Plus, Search, Server } from '@lucide/vue'
import { useQuery } from '@tanstack/vue-query'
import { useEventListener, useStorage } from '@vueuse/core'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'

import { usePermissions } from '@/features/account/account.queries'
import { nodesQuery } from '@/features/nodes/nodes.queries'
import { serversQuery } from '@/features/servers/servers.queries'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/shared/ui/dialog'
import { SidebarMenuButton } from '@/shared/ui/sidebar'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/shared/ui/tooltip'

import { normalize } from './nav'
import { sections } from './sections'

const RECENT_LIMIT = 5
const RESULT_LIMIT = 8

interface Command {
  key: string
  group: 'recent' | 'actions' | 'pages' | 'servers' | 'nodes'
  label: string
  detail?: string
  icon: LucideIcon
  to: string
}

const { t } = useI18n()
const { can } = usePermissions()
const route = useRoute()
const router = useRouter()

const open = ref(false)
const query = ref('')
const active = ref(0)

useEventListener(window, 'keydown', (event: KeyboardEvent) => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault()
    open.value = !open.value
  }
})

// Listes lues seulement palette ouverte : pas de rafraîchissement en fond sur chaque page.
const { data: servers } = useQuery(computed(() => ({ ...serversQuery, enabled: open.value && can('servers.read') })))
const { data: nodes } = useQuery(computed(() => ({ ...nodesQuery, enabled: open.value && can('nodes.read') })))

/** Derniers serveurs et nœuds ouverts (`server:<id>`, `node:<id>`), du plus récent au plus ancien. */
const recent = useStorage<string[]>('kuro.palette.recent', [])
const UUID = '[0-9a-f-]{36}'
watch(
  () => route.path,
  (path) => {
    const node = path.match(new RegExp(`^/servers/nodes/(${UUID})`))
    const server = node ? null : path.match(new RegExp(`^/servers/(${UUID})`))
    const key = node ? `node:${node[1]}` : server ? `server:${server[1]}` : null
    if (key) recent.value = [key, ...recent.value.filter((item) => item !== key)].slice(0, RECENT_LIMIT)
  },
  { immediate: true },
)

// Une page peut figurer dans plusieurs sections (extensions) : une seule entrée par adresse.
const pages = computed<Command[]>(() => [
  ...new Map(pageEntries.value.map((page) => [page.key, page])).values(),
])

const pageEntries = computed<Command[]>(() =>
  sections
    .filter((section) => !section.permission || can(section.permission))
    .flatMap((section) =>
      section.items.flatMap((item) => {
        if (item.permission && !can(item.permission)) return []
        const entries = item.children
          ? item.children.filter((child) => !child.permission || can(child.permission))
          : item.to && !item.soon
            ? [{ label: item.label, to: item.to }]
            : []
        return entries.map((entry) => ({
          key: `page:${entry.to}`,
          group: 'pages' as const,
          label: t(entry.label),
          detail: t(section.title),
          icon: item.icon,
          to: entry.to,
        }))
      }),
    ),
)

const actions = computed<Command[]>(() =>
  can('servers.create')
    ? [{ key: 'action:new-server', group: 'actions', label: t('servers.create.action'), icon: Plus, to: '/servers/new' }]
    : [],
)

const serverCommands = computed<Command[]>(() =>
  (servers.value ?? []).map((server) => ({
    key: `server:${server.id}`,
    group: 'servers',
    label: server.name,
    detail: `${server.prefab.id} · ${server.node.name}`,
    icon: Server,
    to: `/servers/${server.id}`,
  })),
)

const nodeCommands = computed<Command[]>(() =>
  (nodes.value ?? []).map((node) => ({
    key: `node:${node.id}`,
    group: 'nodes',
    label: node.name,
    detail: t('palette.node'),
    icon: Network,
    to: `/servers/nodes/${node.id}`,
  })),
)

const results = computed<Command[]>(() => {
  const needle = normalize(query.value.trim())
  if (!needle) {
    const known = new Map([...serverCommands.value, ...nodeCommands.value].map((command) => [command.key, command]))
    const recents = recent.value.flatMap((key) => {
      const command = known.get(key)
      return command ? [{ ...command, group: 'recent' as const }] : []
    })
    return [...recents, ...actions.value, ...pages.value]
  }
  const match = (command: Command) => normalize(`${command.label} ${command.detail ?? ''}`).includes(needle)
  return [actions.value, pages.value, serverCommands.value, nodeCommands.value].flatMap((list) =>
    list.filter(match).slice(0, RESULT_LIMIT),
  )
})

watch([query, open], () => (active.value = 0))
watch(open, (value) => value || (query.value = ''))

function go(command: Command | undefined) {
  if (!command) return
  open.value = false
  void router.push(command.to)
}

function move(step: number) {
  const count = results.value.length
  if (count > 0) active.value = (active.value + step + count) % count
}

const optionId = (index: number) => `palette-option-${index}`
</script>

<template>
  <Tooltip>
    <TooltipTrigger as-child>
      <SidebarMenuButton
        class="size-10 justify-center p-0 text-muted-foreground hover:bg-muted hover:text-foreground [&>svg]:size-5"
        :aria-label="t('palette.open')"
        @click="open = true"
      >
        <Search />
      </SidebarMenuButton>
    </TooltipTrigger>
    <TooltipContent side="right">{{ t('palette.open') }} <kbd class="ml-1 font-mono text-xs">{{ t('palette.shortcut') }}</kbd></TooltipContent>
  </Tooltip>

  <Dialog v-model:open="open">
    <DialogContent class="top-[20%] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-xl" :show-close-button="false">
      <DialogTitle class="sr-only">{{ t('palette.title') }}</DialogTitle>
      <DialogDescription class="sr-only">{{ t('palette.help') }}</DialogDescription>
      <div class="flex items-center gap-2 border-b px-4">
        <Search class="size-4 shrink-0 text-muted-foreground" />
        <input
          v-model="query"
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-results"
          :aria-activedescendant="results.length > 0 ? optionId(active) : undefined"
          :aria-label="t('palette.title')"
          :placeholder="t('palette.placeholder')"
          class="h-12 w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          @keydown.down.prevent="move(1)"
          @keydown.up.prevent="move(-1)"
          @keydown.enter.prevent="go(results[active])"
        >
      </div>
      <ul id="palette-results" role="listbox" :aria-label="t('palette.title')" class="max-h-80 overflow-y-auto p-2">
        <li
          v-for="(command, index) in results"
          :id="optionId(index)"
          :key="`${command.group}-${command.key}`"
          role="option"
          :aria-selected="index === active"
          class="flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2 text-sm"
          :class="index === active ? 'bg-muted' : undefined"
          @mousemove="active = index"
          @click="go(command)"
        >
          <component :is="command.icon" class="size-4 shrink-0 text-muted-foreground" />
          <span class="min-w-0 flex-1 truncate font-medium">{{ command.label }}</span>
          <span v-if="command.detail" class="shrink-0 truncate text-xs text-muted-foreground">{{ command.detail }}</span>
          <span class="shrink-0 text-xs text-muted-foreground">{{ t(`palette.groups.${command.group}`) }}</span>
        </li>
        <li v-if="results.length === 0" class="px-3 py-6 text-center text-sm text-muted-foreground">{{ t('palette.empty') }}</li>
      </ul>
    </DialogContent>
  </Dialog>
</template>
