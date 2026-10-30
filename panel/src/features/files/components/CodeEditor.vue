<script setup lang="ts">
// Éditeur de texte (CodeMirror 6) aux couleurs du panel. Le contenu est lu à la demande
// (`content()`) ou suivi par `input` : le parent décide quand enregistrer, `Ctrl+S` le lui
// demande. `extensions` ajoute des comportements (autocomplétion, diagnostics du Studio).
import { indentWithTab } from '@codemirror/commands'
import { syntaxHighlighting } from '@codemirror/language'
import { type Diagnostic, setDiagnostics } from '@codemirror/lint'
import { EditorSelection, EditorState, type Extension } from '@codemirror/state'
import { keymap } from '@codemirror/view'
import { basicSetup, EditorView } from 'codemirror'
import { onBeforeUnmount, onMounted, useTemplateRef } from 'vue'

import { highlight, languageOf } from '../language'

const props = defineProps<{ initial: string; format?: string; label: string; readonly?: boolean; extensions?: Extension[] }>()
const emit = defineEmits<{ change: [dirty: boolean]; save: []; input: [text: string] }>()

const container = useTemplateRef<HTMLDivElement>('container')
let view: EditorView | null = null
let saved = props.initial

const theme = EditorView.theme({
  '&': { height: '100%', backgroundColor: 'var(--card)', color: 'var(--card-foreground)', fontSize: '13px' },
  '.cm-scroller': { fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace' },
  '.cm-gutters': { backgroundColor: 'var(--card)', color: 'var(--muted-foreground)', border: 'none' },
  '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'color-mix(in oklch, var(--accent) 60%, transparent)' },
  '.cm-cursor': { borderLeftColor: 'var(--foreground)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
    backgroundColor: 'color-mix(in oklch, var(--primary) 30%, transparent)',
  },
  '.cm-panels': { backgroundColor: 'var(--popover)', color: 'var(--popover-foreground)' },
  '.cm-tooltip': { backgroundColor: 'var(--popover)', color: 'var(--popover-foreground)', border: '1px solid var(--border)', borderRadius: '6px' },
  '.cm-tooltip-autocomplete ul li[aria-selected]': { backgroundColor: 'var(--accent)', color: 'var(--accent-foreground)' },
  '.cm-completionDetail': { color: 'var(--muted-foreground)', fontStyle: 'normal', marginLeft: '0.75em' },
})

function state(text: string) {
  return EditorState.create({
    doc: text,
    extensions: [
      basicSetup,
      keymap.of([
        indentWithTab,
        {
          key: 'Mod-s',
          preventDefault: true,
          run: () => {
            emit('save')
            return true
          },
        },
      ]),
      languageOf(props.format),
      syntaxHighlighting(highlight),
      theme,
      EditorState.readOnly.of(props.readonly ?? false),
      EditorView.contentAttributes.of({ 'aria-label': props.label }),
      EditorView.updateListener.of((update) => {
        if (!update.docChanged) return
        const text = update.state.doc.toString()
        emit('change', text !== saved)
        emit('input', text)
      }),
      ...(props.extensions ?? []),
    ],
  })
}

onMounted(() => {
  if (container.value) view = new EditorView({ state: state(props.initial), parent: container.value })
})
onBeforeUnmount(() => view?.destroy())

defineExpose({
  content: () => view?.state.doc.toString() ?? saved,
  /** Le contenu enregistré (ou rechargé) devient la référence : plus rien n'est modifié. */
  reset(text: string) {
    saved = text
    view?.setState(state(text))
    emit('change', false)
  },
  /** Problèmes affichés dans la marge et soulignés (positions dans le texte courant). */
  setDiagnostics(diagnostics: Diagnostic[]) {
    if (view) view.dispatch(setDiagnostics(view.state, diagnostics))
  },
  /** Place le curseur à `position` et l'amène à l'écran. */
  reveal(position: number) {
    if (!view) return
    const at = Math.min(position, view.state.doc.length)
    view.dispatch({ selection: EditorSelection.cursor(at), scrollIntoView: true })
    view.focus()
  },
  markSaved(text: string) {
    saved = text
    emit('change', view?.state.doc.toString() !== text)
  },
})
</script>

<template>
  <div ref="container" class="min-h-0 flex-1 overflow-hidden" />
</template>
