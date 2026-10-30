<script setup lang="ts">
// Actions déclarées par le prefab (ADR-0039) : une commande typée lancée dans le conteneur du
// service en marche. Les paramètres sont saisis dans un dialogue, validés par l'API contre leur
// déclaration ; le code de sortie et la sortie (tronquée par le nœud) sont affichés. Une action
// planifiable propose une tâche préremplie.
import { CalendarClock, Play } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import { usePrefabLocale } from '@/features/prefabs/composables/usePrefabLocale'
import { type ActionDecl, type Field, schedulableActions } from '@/features/prefabs/manifest'
import { usePrefab } from '@/features/prefabs/prefabs.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'
import { Switch } from '@/shared/ui/switch'

import { type ActionResult, type Server, useServersApi } from '../servers.api'

const props = defineProps<{ server: Server }>()
const { t } = useI18n()
const { can } = usePermissions()
const { data: prefab } = usePrefab(() => props.server.prefab.id, () => props.server.prefab.version)
const locale = usePrefabLocale(() => props.server.prefab)
const api = useServersApi()
const { handleError } = useApiError()

const actions = computed<[string, ActionDecl][]>(() => Object.entries(prefab.value?.manifest.actions ?? {}))
const schedulable = computed(() => new Set(schedulableActions(prefab.value?.manifest)))
const running = computed(() => props.server.state === 'running' || props.server.state === 'starting')

const open = ref<string | null>(null)
const values = ref<Record<string, unknown>>({})
const errors = ref<FieldErrors>({})
const pending = ref(false)
const result = ref<ActionResult | null>(null)

const current = computed<[string, ActionDecl] | undefined>(() => actions.value.find(([key]) => key === open.value))

/** Valeur initiale d'un paramètre : celle par défaut, sinon vide selon son type. */
function initial(field: Field): unknown {
  if (field.default !== undefined && field.default !== null) return field.default
  if (field.type === 'bool') return false
  if (field.type === 'enum') return field.options?.[0] ?? ''
  return ''
}

watch(open, (key) => {
  const action = actions.value.find(([k]) => k === key)?.[1]
  values.value = Object.fromEntries(Object.entries(action?.params ?? {}).map(([name, field]) => [name, initial(field)]))
  errors.value = {}
  result.value = null
})

/** Paramètres envoyés : un champ vide non obligatoire est omis (sa valeur par défaut s'applique). */
function params(action: ActionDecl): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const [name, field] of Object.entries(action.params ?? {})) {
    const value = values.value[name]
    if (value === '' || value === undefined) continue
    out[name] = field.type === 'int' || field.type === 'float' ? Number(value) : value
  }
  return out
}

async function run() {
  if (!current.value) return
  const [key, action] = current.value
  pending.value = true
  errors.value = {}
  result.value = null
  try {
    result.value = await api.runAction(props.server.id, key, params(action))
    if (result.value.exit_code === 0) toast.success(t('servers.actions.done', { action: locale.actionLabel(key) }))
  } catch (failure) {
    const fields = Object.keys(action.params ?? {}).map((name) => `params.${name}`)
    const raw = handleError(failure, fields)
    errors.value = Object.fromEntries(Object.entries(raw).map(([field, message]) => [field.replace(/^params\./, ''), message]))
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <Card v-if="actions.length && can('servers.actions.run')">
    <CardHeader>
      <CardTitle>{{ t('servers.actions.title') }}</CardTitle>
      <CardDescription>{{ t('servers.actions.help') }}</CardDescription>
    </CardHeader>
    <CardContent class="flex flex-wrap gap-2">
      <div v-for="[key, action] in actions" :key="key" class="flex items-center gap-1">
        <Button variant="outline" size="sm" :disabled="!running" :title="locale.actionHelp(key)" @click="open = key">
          <Play />{{ locale.actionLabel(key) }}
        </Button>
        <Button
          v-if="schedulable.has(key) && can('servers.schedules.manage')"
          variant="ghost"
          size="icon-sm"
          :aria-label="t('servers.actions.schedule', { action: locale.actionLabel(key) })"
          as-child
        >
          <RouterLink :to="{ path: `/servers/${server.id}/schedules/new`, query: { action: key } }"><CalendarClock /></RouterLink>
        </Button>
        <span class="sr-only">{{ Object.keys(action.params ?? {}).length }}</span>
      </div>
      <p v-if="!running" class="basis-full text-xs text-muted-foreground">{{ t('servers.actions.stopped') }}</p>
    </CardContent>
  </Card>

  <Dialog :open="open !== null" @update:open="(value) => { if (!value) open = null }">
    <DialogContent v-if="current" class="sm:max-w-lg">
      <DialogHeader>
        <DialogTitle>{{ locale.actionLabel(current[0]) }}</DialogTitle>
        <DialogDescription>{{ locale.actionHelp(current[0]) ?? t('servers.actions.dialogHelp') }}</DialogDescription>
      </DialogHeader>
      <form class="grid gap-4" @submit.prevent="run">
        <div v-for="[name, field] in Object.entries(current[1].params ?? {})" :key="name" class="grid gap-1.5">
          <Label :for="`action-param-${name}`">{{ locale.actionParamLabel(current[0], name) }}</Label>
          <div v-if="field.type === 'bool'" class="flex items-center gap-2">
            <Switch :id="`action-param-${name}`" :model-value="Boolean(values[name])" @update:model-value="(value) => (values[name] = value)" />
          </div>
          <Select v-else-if="field.type === 'enum'" :model-value="String(values[name] ?? '')" @update:model-value="(value) => (values[name] = value)">
            <SelectTrigger :id="`action-param-${name}`" class="w-full" :aria-invalid="!!errors[name]">
              <SelectValue>{{ locale.optionLabel(name, values[name]) }}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem v-for="option in field.options ?? []" :key="String(option)" :value="String(option)">
                {{ locale.optionLabel(name, option) }}
              </SelectItem>
            </SelectContent>
          </Select>
          <Input
            v-else
            :id="`action-param-${name}`"
            :model-value="String(values[name] ?? '')"
            :type="field.type === 'int' || field.type === 'float' ? 'number' : 'text'"
            :min="field.min ?? undefined"
            :max="field.max ?? undefined"
            :required="field.required"
            class="font-mono"
            autocomplete="off"
            spellcheck="false"
            :aria-invalid="!!errors[name]"
            @update:model-value="(value) => (values[name] = value)"
          />
          <p v-if="errors[name]" class="text-xs text-destructive">{{ errors[name] }}</p>
        </div>
        <p v-if="!Object.keys(current[1].params ?? {}).length" class="text-sm text-muted-foreground">{{ t('servers.actions.noParams') }}</p>
        <div v-if="result" class="grid gap-1.5" role="status">
          <span class="text-sm" :class="result.exit_code === 0 ? 'text-muted-foreground' : 'text-destructive'">
            {{ t('servers.actions.exitCode', { code: result.exit_code }) }}
          </span>
          <pre class="max-h-64 overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs whitespace-pre-wrap">{{ result.output || t('servers.actions.noOutput') }}</pre>
          <span v-if="result.truncated" class="text-xs text-muted-foreground">{{ t('servers.actions.truncated') }}</span>
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" @click="open = null">{{ t('common.close') }}</Button>
          <Button type="submit" :disabled="pending || !running">{{ t(pending ? 'servers.actions.running' : 'servers.actions.run') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
