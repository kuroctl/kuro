<script setup lang="ts">
// Import d'un `docker-compose.yml` ou d'un egg Pterodactyl (ADR-0027) : collé ou choisi,
// converti par l'API (rien n'est enregistré), avec ce qui a été adapté, ignoré ou refusé ; puis
// création du brouillon.
import { AlertTriangle, Ban, CheckCircle2, FileUp, Info } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Textarea } from '@/shared/ui/textarea'

import { type ComposeConversion, useStudioApi } from '../studio.api'
import { useInvalidateStudio } from '../studio.queries'

/** Tailles maximales, comme l'API. */
const MAX_BYTES = { compose: 256 * 1024, egg: 512 * 1024 }
const ACCEPT = { compose: '.yml,.yaml', egg: '.json' }

const props = withDefaults(defineProps<{ open: boolean; source?: 'compose' | 'egg' }>(), { source: 'compose' })
const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const { t } = useI18n()
const router = useRouter()
const api = useStudioApi()
const invalidate = useInvalidateStudio()
const { handleError } = useApiError()
const text = ref('')
const id = ref('')
const conversion = ref<ComposeConversion>()
const pending = ref(false)
const input = ref<HTMLInputElement>()

watch(
  () => props.open,
  (open) => {
    if (!open) return
    text.value = ''
    id.value = ''
    conversion.value = undefined
  },
)
// Le texte change : l'aperçu ne correspond plus.
watch([text, id], () => (conversion.value = undefined))

const groups = computed(() => {
  const notes = conversion.value?.notes ?? []
  return (['refused', 'adapted', 'ignored'] as const)
    .map((kind) => ({ kind, notes: notes.filter((note) => note.kind === kind) }))
    .filter((group) => group.notes.length)
})
const icons = { refused: Ban, adapted: AlertTriangle, ignored: Info }

async function picked(event: Event) {
  const target = event.target as HTMLInputElement
  const file = target.files?.[0]
  target.value = ''
  if (!file) return
  if (file.size > MAX_BYTES[props.source]) {
    toast.error(t(`studio.${props.source}.tooLarge`))
    return
  }
  text.value = await file.text()
}

async function convert() {
  pending.value = true
  try {
    const wanted = id.value.trim() || undefined
    conversion.value =
      props.source === 'egg' ? await api.convertEgg(text.value, wanted) : await api.convertCompose(text.value, wanted)
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}

async function create() {
  pending.value = true
  try {
    const draft = await api.create({ [props.source]: { text: text.value, id: id.value.trim() || undefined } })
    await invalidate.drafts()
    emit('update:open', false)
    await router.push(`/prefabs/studio/${draft.id}`)
  } catch (error) {
    handleError(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
      <DialogHeader>
        <DialogTitle>{{ t(`studio.${source}.title`) }}</DialogTitle>
        <DialogDescription>{{ t(`studio.${source}.help`) }}</DialogDescription>
      </DialogHeader>
      <form class="grid gap-4" @submit.prevent="conversion ? create() : convert()">
        <div class="grid gap-2">
          <div class="flex items-center justify-between gap-2">
            <Label for="compose-text">{{ t(`studio.${source}.file`) }}</Label>
            <input ref="input" type="file" :accept="ACCEPT[source]" class="hidden" :aria-label="t('studio.compose.choose')" @change="picked">
            <Button type="button" size="sm" variant="ghost" @click="input?.click()"><FileUp />{{ t('studio.compose.choose') }}</Button>
          </div>
          <Textarea id="compose-text" v-model="text" rows="10" spellcheck="false" class="font-mono text-xs" :placeholder="t(`studio.${source}.placeholder`)" />
        </div>
        <div class="grid gap-2">
          <Label for="compose-id">{{ t('studio.compose.id') }}</Label>
          <Input id="compose-id" v-model="id" autocomplete="off" spellcheck="false" :placeholder="t(`studio.${source}.idPlaceholder`)" />
        </div>

        <section v-if="conversion" class="grid gap-3 rounded-lg border p-3" :aria-label="t('studio.compose.result')">
          <p class="flex items-center gap-2 text-sm font-semibold">
            <CheckCircle2 v-if="conversion.check.valid" class="size-4 text-success" />
            <AlertTriangle v-else class="size-4 text-warning" />
            {{ conversion.check.valid ? t('studio.compose.valid', { id: conversion.id }) : t('studio.compose.toFix', { id: conversion.id }) }}
          </p>
          <div v-for="group in groups" :key="group.kind" class="grid gap-1">
            <p class="flex items-center gap-1.5 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
              <component :is="icons[group.kind]" class="size-3.5" />{{ t(`studio.compose.kinds.${group.kind}`, group.notes.length) }}
            </p>
            <ul class="grid gap-0.5">
              <li v-for="note in group.notes" :key="`${note.path}:${note.message}`" class="font-mono text-xs">
                <span class="text-foreground">{{ note.path }}</span><span class="text-muted-foreground">{{ ` : ${note.message}` }}</span>
              </li>
            </ul>
          </div>
          <ul v-if="conversion.check.issues.length" class="grid gap-0.5">
            <li v-for="issue in conversion.check.issues" :key="`${issue.path}:${issue.message}`" class="font-mono text-xs text-warning">
              {{ `${issue.path} : ${issue.message}` }}
            </li>
          </ul>
        </section>

        <DialogFooter>
          <Button type="button" variant="ghost" @click="emit('update:open', false)">{{ t('common.cancel') }}</Button>
          <Button v-if="!conversion" type="submit" :disabled="pending || !text.trim()">{{ t('studio.compose.convert') }}</Button>
          <Button v-else type="submit" :disabled="pending">{{ t('studio.compose.create') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
