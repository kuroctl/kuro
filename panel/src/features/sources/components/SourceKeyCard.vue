<script setup lang="ts">
// Clé d'API d'une source (ADR-0029) : vérifiée auprès de la source avant d'être enregistrée,
// chiffrée, et jamais renvoyée : le champ reste vide une fois la clé en place.
import { ExternalLink, KeyRound, Trash2 } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/shared/ui/alert-dialog'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { type ContentSource, useSourcesApi } from '../sources.api'
import { useInvalidateSources } from '../sources.queries'

const props = defineProps<{
  source: ContentSource
  /** Page où l'administrateur crée sa clé. */
  consoleUrl: string
}>()

const { t, locale } = useI18n()
const api = useSourcesApi()
const invalidate = useInvalidateSources()
const { handleError } = useApiError()

const name = computed(() => t(`content.sources.${props.source.provider}`))
const host = computed(() => new URL(props.consoleUrl).host)
const fieldId = computed(() => `${props.source.provider}-key`)
const since = computed(() =>
  props.source.updated_at
    ? new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(props.source.updated_at))
    : undefined,
)
const key = ref('')
const errors = ref<FieldErrors>({})
const pending = ref<'save' | 'remove'>()
const confirmRemove = ref(false)

async function save() {
  pending.value = 'save'
  errors.value = {}
  try {
    await api.setKey(props.source.provider, key.value.trim())
    key.value = ''
    toast.success(t('sources.key.saved', { name: name.value }))
    await invalidate()
  } catch (failure) {
    errors.value = handleError(failure, ['api_key'])
  } finally {
    pending.value = undefined
  }
}

async function remove() {
  pending.value = 'remove'
  try {
    await api.removeKey(props.source.provider)
    confirmRemove.value = false
    toast.success(t('sources.key.removed', { name: name.value }))
    await invalidate()
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = undefined
  }
}
</script>

<template>
  <Card role="region" :aria-label="name">
    <CardHeader>
      <CardTitle class="flex items-center gap-2">
        {{ name }}
        <Badge v-if="source.configured" variant="success">{{ t('sources.active') }}</Badge>
        <Badge v-else variant="secondary">{{ t('sources.noKey') }}</Badge>
      </CardTitle>
      <CardDescription>{{ t(`sources.help.${source.provider}`) }}</CardDescription>
    </CardHeader>
    <CardContent>
      <form class="grid gap-4" @submit.prevent="save">
        <p v-if="since" class="text-sm text-muted-foreground">{{ t('sources.key.since', { date: since }) }}</p>
        <div class="grid gap-1.5">
          <Label :for="fieldId">{{ source.configured ? t('sources.key.replace') : t('sources.key.label') }}</Label>
          <Input :id="fieldId" v-model="key" type="password" autocomplete="off" spellcheck="false" :aria-invalid="!!errors.api_key" />
          <p v-if="errors.api_key" class="text-sm text-destructive">{{ errors.api_key }}</p>
          <p v-else class="text-xs text-muted-foreground">
            {{ t('sources.key.where', { name }) }}
            <a :href="consoleUrl" target="_blank" rel="noopener noreferrer" class="inline-flex items-center gap-1 underline">
              {{ host }}<ExternalLink class="size-3" />
            </a>
          </p>
        </div>
        <div class="flex flex-wrap justify-end gap-2">
          <Button v-if="source.configured" type="button" variant="outline" :disabled="!!pending" @click="confirmRemove = true">
            <Trash2 />{{ t('sources.key.remove') }}
          </Button>
          <Button type="submit" :disabled="!!pending || !key.trim()"><KeyRound />{{ t('sources.key.save') }}</Button>
        </div>
      </form>
    </CardContent>

    <AlertDialog v-model:open="confirmRemove">
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{{ t('sources.key.removeTitle', { name }) }}</AlertDialogTitle>
          <AlertDialogDescription>{{ t('sources.key.removeHelp', { name }) }}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
          <!-- Bouton explicite : AlertDialogAction fermerait avant le gestionnaire. -->
          <Button variant="destructive" :disabled="!!pending" @click="remove">{{ t('sources.key.remove') }}</Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </Card>
</template>
