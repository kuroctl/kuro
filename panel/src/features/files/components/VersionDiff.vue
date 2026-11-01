<script setup lang="ts">
// Comparaison côte à côte (CodeMirror Merge) : la version gardée à gauche, l'actuelle à droite,
// en lecture seule. Recréée quand l'une des deux change.
import { MergeView } from '@codemirror/merge'
import { syntaxHighlighting } from '@codemirror/language'
import { EditorState } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { onBeforeUnmount, onMounted, useTemplateRef, watch } from 'vue'

import { highlight, languageOf } from '../language'
import { theme } from '../theme'

const props = defineProps<{ before: string; after: string; format?: string; beforeLabel: string; afterLabel: string }>()

const container = useTemplateRef<HTMLDivElement>('container')
let view: MergeView | null = null

function side(doc: string, label: string) {
  return {
    doc,
    extensions: [
      EditorView.lineWrapping,
      languageOf(props.format),
      syntaxHighlighting(highlight),
      theme,
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
      EditorView.contentAttributes.of({ 'aria-label': label }),
    ],
  }
}

function render() {
  view?.destroy()
  if (!container.value) return
  view = new MergeView({
    a: side(props.before, props.beforeLabel),
    b: side(props.after, props.afterLabel),
    parent: container.value,
    collapseUnchanged: { margin: 3, minSize: 6 },
    gutter: true,
  })
}

onMounted(render)
watch(() => [props.before, props.after, props.format], render)
onBeforeUnmount(() => view?.destroy())
</script>

<template>
  <div ref="container" data-slot="version-diff" class="min-h-0 overflow-auto rounded-lg border" />
</template>
