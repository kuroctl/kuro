<script setup lang="ts">
// Champ de commande de la console : Entrée envoie, flèches haut et bas parcourent l'historique.
import { ChevronRight, SendHorizontal } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'

import { type Server, useServersApi } from '../servers.api'
import { canCommand } from '../state'

const props = defineProps<{ server: Server }>()

/** Commandes gardées dans l'historique de la session. */
const HISTORY = 50

const { t } = useI18n()
const { command } = useServersApi()
const { handleError } = useApiError()

const line = ref('')
const history = ref<string[]>([])
const cursor = ref<number | null>(null)
const error = ref<string>()
const pending = ref(false)
const enabled = computed(() => canCommand(props.server.state))

async function submit() {
  const value = line.value
  if (!value.trim() || pending.value) return
  pending.value = true
  error.value = undefined
  try {
    await command(props.server.id, value)
    if (history.value.at(-1) !== value) history.value = [...history.value, value].slice(-HISTORY)
    line.value = ''
    cursor.value = null
  } catch (caught) {
    error.value = handleError(caught, ['line']).line
  } finally {
    pending.value = false
  }
}

function browse(direction: -1 | 1) {
  const entries = history.value
  if (entries.length === 0) return
  if (direction === -1) {
    cursor.value = cursor.value === null ? entries.length - 1 : Math.max(0, cursor.value - 1)
  } else if (cursor.value !== null) {
    cursor.value = cursor.value + 1 < entries.length ? cursor.value + 1 : null
  }
  line.value = cursor.value === null ? '' : (entries[cursor.value] ?? '')
}
</script>

<template>
  <form class="grid gap-1 border-t border-white/7 bg-console px-3.5 text-console-foreground" @submit.prevent="submit">
    <div class="flex items-center gap-2">
      <ChevronRight class="size-4 shrink-0 text-[oklch(0.86_0.09_272)]" aria-hidden="true" />
      <Input
        v-model="line"
        class="h-10 rounded-none border-0 bg-transparent px-0 font-mono text-[12.5px] placeholder:text-[oklch(0.6_0.02_265)] hover:border-0 focus-visible:ring-0"
        autocomplete="off"
        spellcheck="false"
        :disabled="!enabled"
        :aria-label="t('servers.console.commandLabel')"
        :aria-invalid="error ? true : undefined"
        :placeholder="enabled ? t('servers.console.commandPlaceholder') : t('servers.console.notRunning')"
        @keydown.up.prevent="browse(-1)"
        @keydown.down.prevent="browse(1)"
      />
      <Button type="submit" size="icon-sm" variant="ghost" class="text-console-foreground hover:bg-white/10 hover:text-console-foreground" :disabled="!enabled || pending || !line.trim()" :aria-label="t('servers.console.send')">
        <SendHorizontal />
      </Button>
    </div>
    <p v-if="error" class="px-1 pb-2 text-xs text-destructive">{{ error }}</p>
  </form>
</template>
