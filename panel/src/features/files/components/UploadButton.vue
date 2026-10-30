<script setup lang="ts">
// Bouton d'envoi de fichier : choisit un fichier, refuse au-delà de la taille maximale, affiche
// la progression. L'envoi lui-même est fait par le parent (`send`).
import { Upload } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { formatBytes } from '@/shared/lib/format'
import { Button } from '@/shared/ui/button'

/** 256 Mio, comme l'API. */
const MAX_UPLOAD_BYTES = 256 * 1024 * 1024

const props = defineProps<{
  label: string
  accept?: string
  send: (file: File, progress: (ratio: number) => void) => Promise<void>
}>()

const { t, locale } = useI18n()
const input = ref<HTMLInputElement>()
const progress = ref<number | null>(null)

async function picked(event: Event) {
  const target = event.target as HTMLInputElement
  const file = target.files?.[0]
  target.value = ''
  if (!file) return
  if (file.size > MAX_UPLOAD_BYTES) {
    toast.error(t('files.upload.tooLarge', { max: formatBytes(MAX_UPLOAD_BYTES, locale.value) }))
    return
  }
  progress.value = 0
  try {
    await props.send(file, (ratio) => (progress.value = ratio))
  } finally {
    progress.value = null
  }
}
</script>

<template>
  <input ref="input" type="file" class="hidden" :accept="accept" :aria-label="label" @change="picked">
  <Button size="sm" variant="outline" :disabled="progress !== null" @click="input?.click()">
    <Upload />
    <template v-if="progress === null">{{ label }}</template>
    <template v-else>{{ t('files.upload.progress', { percent: Math.round(progress * 100) }) }}</template>
  </Button>
</template>
