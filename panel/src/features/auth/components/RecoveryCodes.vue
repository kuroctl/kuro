<script setup lang="ts">
// Codes de secours du TOTP, montrés une seule fois (activation, remplacement, régénération) :
// copie, téléchargement, et confirmation qu'ils sont gardés avant de poursuivre.
import { Copy, Download } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { saveBlob } from '@/shared/lib/download'
import { Button } from '@/shared/ui/button'
import { Checkbox } from '@/shared/ui/checkbox'

const props = defineProps<{
  codes: string[]
  /** Compte concerné, rappelé dans le fichier téléchargé. */
  account: string
}>()
const emit = defineEmits<{ done: [] }>()

const { t } = useI18n()
const saved = ref(false)

async function copy() {
  try {
    await navigator.clipboard.writeText(props.codes.join('\n'))
    toast.success(t('auth.recoveryCodes.copied'))
  } catch {
    toast.error(t('auth.recoveryCodes.copyFailed'))
  }
}

function download() {
  const text = [t('auth.recoveryCodes.fileHeader', { account: props.account }), '', ...props.codes, ''].join('\n')
  saveBlob(new Blob([text], { type: 'text/plain;charset=utf-8' }), 'kuro-recovery-codes.txt')
}
</script>

<template>
  <div class="flex flex-col gap-4">
    <p class="text-sm text-muted-foreground">{{ t('auth.recoveryCodes.help') }}</p>
    <ul class="grid grid-cols-2 gap-x-4 gap-y-1 rounded-lg border bg-muted px-4 py-3 font-mono text-sm" data-testid="recovery-codes">
      <li v-for="code in codes" :key="code">{{ code }}</li>
    </ul>
    <div class="flex gap-2">
      <Button type="button" variant="outline" size="sm" @click="copy"><Copy />{{ t('auth.recoveryCodes.copy') }}</Button>
      <Button type="button" variant="outline" size="sm" @click="download"><Download />{{ t('auth.recoveryCodes.download') }}</Button>
    </div>
    <label class="flex items-center gap-2 text-sm">
      <Checkbox :model-value="saved" @update:model-value="(value) => (saved = value === true)" />
      {{ t('auth.recoveryCodes.saved') }}
    </label>
    <Button type="button" :disabled="!saved" @click="emit('done')">{{ t('auth.recoveryCodes.done') }}</Button>
  </div>
</template>
