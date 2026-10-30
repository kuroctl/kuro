<script setup lang="ts">
import { Construction } from '@lucide/vue'
import { useI18n } from 'vue-i18n'

import PageHeader from './PageHeader.vue'

defineProps<{ title: string }>()
const { t } = useI18n()
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="title" />
    <div class="flex flex-1 flex-col items-center justify-center gap-3 p-10 text-center text-muted-foreground">
      <Construction class="size-10 opacity-60" />
      <p class="text-sm">{{ t('common.comingSoon') }}</p>
    </div>
  </div>
</template>
