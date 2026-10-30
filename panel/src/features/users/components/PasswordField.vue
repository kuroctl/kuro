<script setup lang="ts">
// Mot de passe à transmettre à un admin : saisi, ou généré puis copié.
import { Copy, Eye, EyeOff, Wand2 } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { generatePassword } from '@/shared/lib/password'
import { Button } from '@/shared/ui/button'
import { Input } from '@/shared/ui/input'

const password = defineModel<string>({ required: true })
defineProps<{ id: string; invalid?: boolean }>()

const { t } = useI18n()
const visible = ref(false)

function generate() {
  password.value = generatePassword()
  visible.value = true
}

async function copy() {
  try {
    await navigator.clipboard.writeText(password.value)
    toast.success(t('users.password.copied'))
  } catch {
    toast.error(t('users.password.copyFailed'))
  }
}
</script>

<template>
  <div class="flex gap-1.5">
    <Input
      :id="id"
      v-model="password"
      :type="visible ? 'text' : 'password'"
      autocomplete="new-password"
      spellcheck="false"
      class="font-mono"
      :aria-invalid="invalid"
    />
    <Button type="button" size="icon" variant="outline" :aria-label="visible ? t('users.password.hide') : t('users.password.show')" @click="visible = !visible">
      <EyeOff v-if="visible" /><Eye v-else />
    </Button>
    <Button type="button" size="icon" variant="outline" :aria-label="t('users.password.generate')" @click="generate"><Wand2 /></Button>
    <Button type="button" size="icon" variant="outline" :disabled="!password" :aria-label="t('users.password.copy')" @click="copy"><Copy /></Button>
  </div>
</template>
