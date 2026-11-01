<script setup lang="ts">
// Code TOTP à 6 chiffres : focus à l'affichage, chiffres seuls, et `complete` dès le sixième
// chiffre, pour valider sans passer par le bouton (qui reste là).
import { onMounted, useTemplateRef, watch } from 'vue'
import { REGEXP_ONLY_DIGITS } from 'vue-input-otp'

import { InputOTP, InputOTPGroup, InputOTPSeparator, InputOTPSlot } from '@/shared/ui/input-otp'

const props = defineProps<{ id: string; disabled?: boolean; invalid?: boolean }>()
const emit = defineEmits<{ complete: [code: string] }>()
const code = defineModel<string>({ required: true })

/** Longueur imposée par l'API (`auth::totp`). */
const LENGTH = 6

const root = useTemplateRef<HTMLElement>('root')

// Focus posé à la main, pas par l'attribut `autofocus` : le champ apparaît souvent désactivé
// (requête de l'étape précédente encore en cours), et dans un dialogue le piège à focus de
// reka-ui rend le focus au conteneur quand l'étape précédente disparaît. Il est donc posé après
// le rendu, et de nouveau dès que le champ redevient actif (code refusé, à ressaisir).
function focus() {
  setTimeout(() => root.value?.querySelector('input')?.focus())
}
onMounted(focus)
watch(
  () => props.disabled,
  (disabled) => {
    if (!disabled) focus()
  },
)
</script>

<template>
  <div ref="root">
    <InputOTP
      :id="id"
      v-model="code"
      :maxlength="LENGTH"
      :pattern="REGEXP_ONLY_DIGITS"
      inputmode="numeric"
      autocomplete="one-time-code"
      :disabled="disabled"
      class="justify-center"
      @complete="(value: string) => emit('complete', value)"
    >
      <InputOTPGroup>
        <InputOTPSlot v-for="index in 3" :key="index" :index="index - 1" class="size-11 text-lg" :aria-invalid="invalid" />
      </InputOTPGroup>
      <InputOTPSeparator />
      <InputOTPGroup>
        <InputOTPSlot v-for="index in 3" :key="index" :index="index + 2" class="size-11 text-lg" :aria-invalid="invalid" />
      </InputOTPGroup>
    </InputOTP>
  </div>
</template>
