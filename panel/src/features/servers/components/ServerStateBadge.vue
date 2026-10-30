<script setup lang="ts">
import { useI18n } from 'vue-i18n'

import { Badge } from '@/shared/ui/badge'

import { type ServerState, stateBadge } from '../state'

defineProps<{ state: ServerState; suspended?: boolean }>()
const { t } = useI18n()
</script>

<template>
  <span class="inline-flex items-center gap-1.5">
    <Badge :variant="stateBadge[state].variant">
      <span class="size-[7px] rounded-full bg-current" :class="{ 'animate-pulse': stateBadge[state].pulse }" aria-hidden="true" />
      {{ t(`servers.state.${state}`) }}
    </Badge>
    <Badge v-if="suspended" variant="destructive">{{ t('servers.suspension.badge') }}</Badge>
  </span>
</template>
