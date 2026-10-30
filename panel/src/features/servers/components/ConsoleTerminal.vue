<script setup lang="ts">
// Terminal en lecture seule (xterm.js) : sortie brute du serveur, couleurs ANSI comprises, aux
// couleurs du thème. Les lignes arrivent par `write`, exposé au parent.
import '@xterm/xterm/css/xterm.css'

import { FitAddon } from '@xterm/addon-fit'
import { Terminal, type ITheme } from '@xterm/xterm'
import { useResizeObserver } from '@vueuse/core'
import { onBeforeUnmount, onMounted, useTemplateRef, watch } from 'vue'

import { useTheme } from '@/shared/theme/useTheme'

const props = defineProps<{ label: string }>()

const container = useTemplateRef<HTMLDivElement>('container')
const mode = useTheme()
let terminal: Terminal | null = null
const fit = new FitAddon()

/** Couleur CSS quelconque (oklch…) convertie en `rgb()` par le navigateur, que xterm comprend. */
function resolve(variable: string): string | undefined {
  const value = getComputedStyle(document.documentElement).getPropertyValue(variable).trim()
  const context = document.createElement('canvas').getContext('2d')
  if (!value || !context) return undefined
  context.fillStyle = value
  context.fillRect(0, 0, 1, 1)
  const [r, g, b] = context.getImageData(0, 0, 1, 1).data
  return `rgb(${r}, ${g}, ${b})`
}

function theme(): ITheme {
  return {
    background: resolve('--console'),
    foreground: resolve('--console-foreground'),
    cursor: resolve('--console'),
    selectionBackground: resolve('--accent'),
  }
}

onMounted(() => {
  if (!container.value) return
  terminal = new Terminal({
    convertEol: true,
    disableStdin: true,
    cursorStyle: 'bar',
    cursorInactiveStyle: 'none',
    fontFamily: "'JetBrains Mono Variable', ui-monospace, monospace",
    fontSize: 12.5,
    lineHeight: 1.4,
    scrollback: 5000,
    screenReaderMode: true,
    theme: theme(),
  })
  terminal.loadAddon(fit)
  terminal.open(container.value)
  terminal.textarea?.setAttribute('aria-label', props.label)
  fit.fit()
})

useResizeObserver(container, () => fit.fit())
// Couleurs relues une fois la classe du thème appliquée au document.
watch(mode, () =>
  requestAnimationFrame(() => {
    if (terminal) terminal.options.theme = theme()
  }),
)
onBeforeUnmount(() => terminal?.dispose())

defineExpose({
  write(lines: string[]) {
    if (lines.length > 0) terminal?.write(`${lines.join('\n')}\n`)
  },
  /** Message de Kuro (pas du serveur), atténué. */
  notice(text: string) {
    terminal?.write(`\x1b[2m${text}\x1b[0m\n`)
  },
})
</script>

<template>
  <div ref="container" class="min-h-0 flex-1 bg-console px-3.5 py-2.5" />
</template>
