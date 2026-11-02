<script setup lang="ts">
// Page de l'extension de démonstration, servie sous `/demo/notes` avec les composants de Kuro.
import { useI18n } from 'vue-i18n'

import PageHeader from '@/shared/layout/PageHeader.vue'

const { t } = useI18n()
</script>

<template>
  <PageHeader :title="t('ext.demo.notes.title')" />
</template>
