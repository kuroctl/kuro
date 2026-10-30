<script setup lang="ts">
// Une image du brouillon (icône, bannière ou vignette) : aperçu, dépôt, retrait. L'API la
// reconnaît à son contenu (PNG, JPEG ou WebP, 2 Mio) ; les dimensions sont celles de la
// convention, l'optimisation se fait après publication.
import { useObjectUrl } from '@vueuse/core'
import { ImagePlus, Trash2 } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import type { PrefabAssetKind } from '@/features/prefabs/prefabs.api'
import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'

import { useStudioApi } from '../studio.api'
import { useDraftAsset, useInvalidateStudio } from '../studio.queries'

/** 2 Mio, comme l'API. */
const MAX_ASSET_BYTES = 2 * 1024 * 1024

const props = defineProps<{ draft: string; kind: PrefabAssetKind; present: boolean }>()
const emit = defineEmits<{ changed: [present: boolean] }>()

const { t } = useI18n()
const api = useStudioApi()
const invalidate = useInvalidateStudio()
const { handleError } = useApiError()
const { data } = useDraftAsset(
  () => props.draft,
  props.kind,
  () => props.present,
)
const src = useObjectUrl(data)
const input = ref<HTMLInputElement>()
const pending = ref(false)

async function picked(event: Event) {
  const target = event.target as HTMLInputElement
  const file = target.files?.[0]
  target.value = ''
  if (!file) return
  if (file.size > MAX_ASSET_BYTES) {
    toast.error(t('studio.assets.tooLarge'))
    return
  }
  pending.value = true
  try {
    await api.putAsset(props.draft, props.kind, file)
    await invalidate.assets(props.draft)
    emit('changed', true)
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}

async function remove() {
  pending.value = true
  try {
    await api.removeAsset(props.draft, props.kind)
    await invalidate.assets(props.draft)
    emit('changed', false)
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <div class="grid gap-3 rounded-xl border bg-card p-4 shadow-card" :data-asset="kind">
    <div class="flex items-baseline justify-between gap-2">
      <p class="text-sm font-semibold">{{ t(`prefabs.detail.assetKinds.${kind}`) }}</p>
      <p class="font-mono text-xs text-muted-foreground">{{ t(`studio.assets.size.${kind}`) }}</p>
    </div>
    <div
      class="flex items-center justify-center overflow-hidden rounded-lg border border-dashed bg-muted/40"
      :class="{ 'aspect-square': kind === 'icon', 'aspect-[8/3]': kind === 'cover', 'aspect-[3/4]': kind === 'thumbnail' }"
    >
      <img v-if="present && src" :src="src" alt="" class="size-full object-cover">
      <p v-else class="px-3 text-center text-xs text-muted-foreground">{{ t('studio.assets.none') }}</p>
    </div>
    <div class="flex flex-wrap gap-2">
      <input ref="input" type="file" accept="image/png,image/jpeg,image/webp" class="hidden" :aria-label="t('studio.assets.choose', { kind: t(`prefabs.detail.assetKinds.${kind}`) })" @change="picked">
      <Button size="sm" variant="outline" :disabled="pending" @click="input?.click()">
        <ImagePlus />{{ present ? t('studio.assets.replace') : t('studio.assets.add') }}
      </Button>
      <Button v-if="present" size="sm" variant="ghost" :disabled="pending" :aria-label="t('studio.assets.remove', { kind: t(`prefabs.detail.assetKinds.${kind}`) })" @click="remove">
        <Trash2 />
      </Button>
    </div>
  </div>
</template>
