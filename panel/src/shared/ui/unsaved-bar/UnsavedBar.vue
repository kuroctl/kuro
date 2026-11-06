<script setup lang="ts">
// Barre des modifications non enregistrées : elle glisse depuis le bas dès que le formulaire
// diffère de ce qui est enregistré, et reste visible pendant le défilement (collée au bas de la
// zone de contenu). Le bouton d'enregistrement soumet le formulaire qui la contient. Tant qu'elle
// est visible, quitter la page (navigation ou fermeture de l'onglet) demande confirmation.
import { useEventListener } from "@vueuse/core"
import { ref } from "vue"
import { useI18n } from "vue-i18n"
import { onBeforeRouteLeave, onBeforeRouteUpdate } from "vue-router"

import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog"
import { Button } from "@/shared/ui/button"

const props = withDefaults(defineProps<{ show: boolean, pending?: boolean, canSave?: boolean, saveLabel?: string }>(), {
  pending: false,
  canSave: true,
  saveLabel: undefined,
})
const emit = defineEmits<{ reset: [] }>()
const { t } = useI18n()

const leaving = ref<((leave: boolean) => void) | null>(null)

// Pendant un enregistrement, la page navigue elle-même (création puis fiche) : pas de question.
function confirmLeave(): Promise<boolean> | boolean {
  if (!props.show || props.pending) return true
  return new Promise((resolve) => (leaving.value = resolve))
}

function answerLeave(leave: boolean) {
  leaving.value?.(leave)
  leaving.value = null
}

onBeforeRouteLeave(confirmLeave)
onBeforeRouteUpdate(confirmLeave)
useEventListener(window, "beforeunload", (event: BeforeUnloadEvent) => {
  if (props.show && !props.pending) event.preventDefault()
})
</script>

<template>
  <div data-slot="unsaved-bar" class="pointer-events-none sticky bottom-6 z-20 flex justify-center">
    <Transition
      enter-active-class="transition duration-200 ease-out motion-reduce:transition-none"
      leave-active-class="transition duration-150 ease-in motion-reduce:transition-none"
      enter-from-class="translate-y-6 opacity-0"
      leave-to-class="translate-y-6 opacity-0"
    >
      <div
        v-if="show"
        role="region"
        :aria-label="t('common.unsaved')"
        class="pointer-events-auto flex flex-wrap items-center gap-3 rounded-xl border bg-popover py-2.5 pr-2.5 pl-4 text-popover-foreground shadow-card"
      >
        <span class="size-2 rounded-full bg-warning" aria-hidden="true" />
        <span class="text-sm font-semibold">{{ t('common.unsaved') }}</span>
        <slot />
        <div class="ml-auto flex gap-2">
          <Button type="button" variant="ghost" size="sm" :disabled="pending" @click="emit('reset')">{{ t('common.reset') }}</Button>
          <Button type="submit" size="sm" :disabled="pending || !canSave">{{ saveLabel ?? t('common.save') }}</Button>
        </div>
      </div>
    </Transition>

    <AlertDialog :open="leaving !== null" @update:open="(open) => !open && answerLeave(false)">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('common.leave.title') }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('common.leave.help') }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button variant="outline" @click="answerLeave(false)">{{ t('common.leave.stay') }}</Button>
          <Button variant="destructive" @click="answerLeave(true)">{{ t('common.leave.discard') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </div>
</template>
