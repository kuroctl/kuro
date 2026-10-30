<script setup lang="ts">
// Essai d'un brouillon : choix d'un nœud de test en ligne, nom du serveur, puis déploiement
// (deux heures, remplace l'essai en cours). Le serveur s'ouvre ensuite comme les autres.
import { Rocket } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { toast } from 'vue-sonner'

import { useNodes } from '@/features/nodes/nodes.queries'
import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

import { useStudioApi } from '../studio.api'
import { useInvalidateStudio } from '../studio.queries'

const props = defineProps<{ open: boolean; draft: string; prefabId: string; replaces: boolean }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()

const { t } = useI18n()
const router = useRouter()
const api = useStudioApi()
const invalidate = useInvalidateStudio()
const { handleError } = useApiError()
const { data: nodes } = useNodes()
const node = ref<string>()
const name = ref('')
const pending = ref(false)

const testNodes = computed(() => (nodes.value ?? []).filter((entry) => entry.trials && entry.status === 'online'))
const label = (id?: string) => {
  const entry = testNodes.value.find((candidate) => candidate.id === id)
  return entry ? (entry.display_name ?? entry.name) : t('studio.trial.pickNode')
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    name.value = t('studio.trial.defaultName', { prefab: props.prefabId })
    node.value = testNodes.value[0]?.id
  },
)

async function start() {
  if (!node.value) return
  pending.value = true
  try {
    const trial = await api.startTrial(props.draft, { node_id: node.value, name: name.value.trim() })
    emit('update:open', false)
    toast.success(t('studio.trial.started'))
    await invalidate.draft(props.draft)
    await router.push(`/servers/${trial.server_id}`)
  } catch (error) {
    handleError(error, ['name', 'node_id'])
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>{{ t('studio.trial.title') }}</DialogTitle>
        <DialogDescription>{{ t('studio.trial.help') }}</DialogDescription>
      </DialogHeader>
      <p v-if="!testNodes.length" class="rounded-md border border-dashed p-3 text-sm text-muted-foreground">{{ t('studio.trial.noNode') }}</p>
      <form v-else class="grid gap-4" @submit.prevent="start">
        <div class="grid gap-2">
          <Label for="trial-node">{{ t('studio.trial.node') }}</Label>
          <Select v-model="node">
            <SelectTrigger id="trial-node" class="w-full"><SelectValue>{{ label(node) }}</SelectValue></SelectTrigger>
            <SelectContent>
              <SelectItem v-for="entry in testNodes" :key="entry.id" :value="entry.id">{{ entry.display_name ?? entry.name }}</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div class="grid gap-2">
          <Label for="trial-name">{{ t('studio.trial.name') }}</Label>
          <Input id="trial-name" v-model="name" maxlength="64" autocomplete="off" />
        </div>
        <p v-if="replaces" class="text-xs text-warning">{{ t('studio.trial.replaces') }}</p>
        <DialogFooter>
          <Button type="button" variant="ghost" @click="emit('update:open', false)">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="pending || !node || !name.trim()"><Rocket />{{ t('studio.trial.start') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
