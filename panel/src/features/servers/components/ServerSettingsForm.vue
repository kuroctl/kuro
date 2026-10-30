<script setup lang="ts">
// Formulaire d'un groupe de réglages d'un serveur : seules les valeurs modifiées sont envoyées ;
// l'API redéploie le serveur (redémarré s'il tournait).
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import PrefabFieldsForm from '@/features/prefabs/components/PrefabFieldsForm.vue'
import { usePrefabForm } from '@/features/prefabs/composables/usePrefabForm'
import type { PrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import type { FieldEntry } from '@/features/prefabs/composables/usePrefabSettings'
import { useApiError } from '@/shared/api/useApiError'
import { UnsavedBar } from '@/shared/ui/unsaved-bar'

import { type Server, useServersApi } from '../servers.api'
import { useInvalidateServers } from '../servers.queries'

const props = defineProps<{
  server: Server
  fields: FieldEntry[]
  locale: PrefabLocale
  needsRestart: (changed: string[]) => boolean
}>()

const { t } = useI18n()
const { updateSettings } = useServersApi()
const { handleError } = useApiError()
const invalidate = useInvalidateServers()
const { form, changes, applyApiErrors, apiFields, locked } = usePrefabForm(
  props.fields,
  props.server.settings,
  'edit',
)
const pending = ref(false)

const changed = computed(() => Object.keys(changes(form.values as Record<string, unknown>)))
const restartWarning = computed(() => props.server.state === 'running' && props.needsRestart(changed.value))

const submit = form.handleSubmit(async (values) => {
  const payload = changes(values)
  if (Object.keys(payload).length === 0) return
  pending.value = true
  try {
    await updateSettings(props.server.id, payload)
    toast.success(t('servers.settings.saved'))
    await invalidate()
  } catch (error) {
    applyApiErrors(handleError(error, apiFields))
  } finally {
    pending.value = false
  }
})
</script>

<template>
  <form class="grid gap-6" @submit.prevent="submit">
    <PrefabFieldsForm :fields="fields" :locale="locale" :locked="locked" />
    <p v-if="restartWarning" class="rounded-lg border border-warning/35 bg-warning/8 px-4 py-3.5 text-sm">
      {{ t('servers.settings.restartWarning') }}
    </p>
    <UnsavedBar :show="changed.length > 0" :pending="pending" @reset="form.resetForm()" />
  </form>
</template>
