<script setup lang="ts">
// Ce que change une mise à jour (ADR-0024) : ce qui la bloque d'abord, puis réglages migrés,
// ports, images et installation relancée.
import { CircleAlert } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { UpgradeBlocker, UpgradePreview } from '../upgrades.api'

const props = defineProps<{ preview: UpgradePreview }>()
const { t, n } = useI18n()

function blocker(entry: UpgradeBlocker): string {
  switch (entry.kind) {
    case 'setting':
      return t('upgrades.blockers.setting', { field: entry.field, reason: t(`upgrades.reasons.${entry.reason}`, entry.reason) })
    case 'content_slot':
      return t('upgrades.blockers.contentSlot', { slot: entry.slot })
    case 'no_free_port':
      return t('upgrades.blockers.noFreePort', { needed: entry.needed, free: entry.free })
    default:
      return t('upgrades.blockers.notDeployable')
    case 'resource_below_minimum':
      return t('upgrades.blockers.resourceBelowMinimum', {
        resource: t(`servers.fields.${entry.resource}`),
        current: n(entry.current),
        minimum: n(entry.minimum),
      })
  }
}

const changes = computed(() => {
  const p = props.preview
  return [
    ...p.renamed.map((r) => t('upgrades.changes.renamed', { from: r.from, to: r.to })),
    ...p.removed.map((key) => t('upgrades.changes.removed', { key })),
    ...p.added.map((key) => t('upgrades.changes.added', { key })),
    ...p.ports_added.map((port) => t('upgrades.changes.portAdded', { port })),
    ...p.ports_removed.map((port) => t('upgrades.changes.portRemoved', { port })),
    ...p.images.map((i) => t('upgrades.changes.image', { service: i.service, to: i.to ?? t('upgrades.changes.none') })),
    ...(p.reinstall ? [t('upgrades.changes.reinstall')] : []),
  ]
})
</script>

<template>
  <div class="grid gap-3 text-sm">
    <ul v-if="preview.blockers.length" class="grid gap-1.5 rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3" role="alert">
      <li v-for="(entry, index) in preview.blockers" :key="index" class="flex gap-2">
        <CircleAlert class="mt-0.5 size-4 shrink-0 text-destructive" />{{ blocker(entry) }}
      </li>
    </ul>
    <ul v-if="changes.length" class="grid list-disc gap-1 pl-5 text-muted-foreground">
      <li v-for="(change, index) in changes" :key="index">{{ change }}</li>
    </ul>
    <p v-else-if="!preview.blockers.length" class="text-muted-foreground">{{ t('upgrades.changes.nothing') }}</p>
  </div>
</template>
