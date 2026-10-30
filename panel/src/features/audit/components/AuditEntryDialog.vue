<script setup lang="ts">
// Détail d'un événement du journal : contexte complet, cible, détails, maillon de la chaîne.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/shared/ui/dialog'

import type { AuditEntry } from '../audit.api'

const props = defineProps<{ entry: AuditEntry | null }>()
const emit = defineEmits<{ close: [] }>()
const { t, locale } = useI18n()

const date = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'full', timeStyle: 'medium' }))
/** Lien vers la cible quand le panel sait l'afficher. */
const targetLink = computed(() =>
  props.entry?.target_kind === 'server' && props.entry.target_id ? `/servers/${props.entry.target_id}` : null,
)
const rows = computed(() => {
  const entry = props.entry
  if (!entry) return []
  return [
    ['seq', String(entry.seq)],
    ['actor', [entry.actor_kind, entry.actor_id].filter(Boolean).join(' · ')],
    ['outcome', t(`audit.outcome.${entry.outcome}`)],
    ['ip', entry.ip ?? ''],
    ['userAgent', entry.user_agent ?? ''],
    ['requestId', entry.request_id ?? ''],
    ['hash', entry.hash],
  ].filter(([, value]) => value !== '')
})
</script>

<template>
  <Dialog :open="entry !== null" @update:open="(open) => !open && emit('close')">
    <DialogContent class="max-h-[90dvh] overflow-y-auto sm:max-w-2xl">
      <DialogHeader>
        <DialogTitle class="font-mono text-base">{{ entry?.action }}</DialogTitle>
        <DialogDescription>{{ entry ? date.format(new Date(entry.occurred_at)) : '' }}</DialogDescription>
      </DialogHeader>
      <dl v-if="entry" class="grid grid-cols-[8rem_1fr] gap-x-4 gap-y-2 text-sm">
        <template v-for="[key, value] in rows" :key="key">
          <dt class="text-muted-foreground">{{ t(`audit.detail.${key}`) }}</dt>
          <dd class="break-all font-mono text-xs leading-5">{{ value }}</dd>
        </template>
        <template v-if="entry.target_kind">
          <dt class="text-muted-foreground">{{ t('audit.detail.target') }}</dt>
          <dd class="break-all font-mono text-xs leading-5">
            <RouterLink v-if="targetLink" :to="targetLink" class="underline underline-offset-2" @click="emit('close')">
              {{ `${entry.target_kind} · ${entry.target_id}` }}
            </RouterLink>
            <template v-else>{{ [entry.target_kind, entry.target_id].filter(Boolean).join(' · ') }}</template>
          </dd>
        </template>
      </dl>
      <div v-if="entry && Object.keys(entry.details).length > 0" class="grid gap-1.5">
        <p class="text-sm text-muted-foreground">{{ t('audit.detail.details') }}</p>
        <pre class="overflow-x-auto rounded-md bg-muted p-3 font-mono text-xs">{{ JSON.stringify(entry.details, null, 2) }}</pre>
      </div>
    </DialogContent>
  </Dialog>
</template>
