<script setup lang="ts">
// Dernière étape de la création : réglages généraux du prefab, démarrage une fois installé (si
// l'admin peut démarrer un serveur), puis déploiement.
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import PrefabFieldsForm from '@/features/prefabs/components/PrefabFieldsForm.vue'
import { usePrefabForm } from '@/features/prefabs/composables/usePrefabForm'
import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { usePrefabSettings } from '@/features/prefabs/composables/usePrefabSettings'
import type { Prefab } from '@/features/prefabs/prefabs.api'
import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Label } from '@/shared/ui/label'
import { Switch } from '@/shared/ui/switch'

import { useServersApi } from '../servers.api'
import { useInvalidateServers } from '../servers.queries'

export interface ServerBasics {
  name: string
  node_id: string
  memory_mb: number
  cpu_percent: number
  disk_mb: number
}

const props = defineProps<{ prefab: Prefab; basics: ServerBasics }>()
const emit = defineEmits<{ back: []; created: [] }>()

const { t } = useI18n()
const router = useRouter()
const { create } = useServersApi()
const { handleError } = useApiError()
const invalidate = useInvalidateServers()
const locale = usePrefabLocale(() => ({ id: props.prefab.id, version: props.prefab.version }))
const { sections } = usePrefabSettings(() => props.prefab.manifest)

// Création : les réglages généraux seulement ; les avancés gardent leur valeur par défaut.
const general = computed(() => sections.value.find((s) => s.key === 'general')?.groups ?? [])
const fields = general.value.flatMap((group) => group.fields)
const { form, changes, applyApiErrors, apiFields } = usePrefabForm(fields, {}, 'create')
const pending = ref(false)
const { can } = usePermissions()
const canStart = computed(() => can('servers.power'))
const start = ref(true)

const submit = form.handleSubmit(async (values) => {
  pending.value = true
  try {
    const server = await create({
      ...props.basics,
      prefab_id: props.prefab.id,
      prefab_version: props.prefab.version,
      settings: changes(values),
      start_when_installed: canStart.value && start.value,
    })
    toast.success(t('servers.create.done', { name: server.name }))
    await invalidate()
    emit('created')
    await router.push(`/servers/${server.id}`)
  } catch (error) {
    applyApiErrors(handleError(error, apiFields))
  } finally {
    pending.value = false
  }
})
</script>

<template>
  <form class="grid gap-6" @submit.prevent="submit">
    <section v-for="group in general" :key="group.key" class="grid gap-4">
      <h3 class="text-sm font-semibold">{{ locale.groupTitle(group.key) }}</h3>
      <PrefabFieldsForm :fields="group.fields" :locale="locale" />
    </section>
    <p v-if="general.length === 0" class="text-sm text-muted-foreground">{{ t('servers.create.noSettings') }}</p>
    <div v-if="canStart" class="flex items-start gap-3 rounded-md border p-3">
      <Switch id="start-when-installed" v-model="start" />
      <div class="grid gap-1">
        <Label for="start-when-installed">{{ t('servers.create.startWhenInstalled') }}</Label>
        <p class="text-sm text-muted-foreground">{{ t('servers.create.startWhenInstalledHelp') }}</p>
      </div>
    </div>
    <div class="flex justify-between">
      <Button type="button" variant="ghost" @click="emit('back')">{{ t('common.back') }}</Button>
      <Button type="submit" :disabled="pending">{{ t('servers.create.submit') }}</Button>
    </div>
  </form>
</template>
