<script setup lang="ts">
// Page d'un groupe du sous-menu de l'extension de démonstration, servie sous `/demo/notes/archive`.
import { useI18n } from 'vue-i18n'

import PageHeader from '@/shared/layout/PageHeader.vue'

const { t } = useI18n()
</script>

<template>
  <PageHeader :title="t('ext.demo.archive.title')" />
</template>
