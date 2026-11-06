<script setup lang="ts">
// Réglages › Tâches (ADR-0062) : tâches de fond de Kuro et des extensions, une ligne par tâche
// avec son dernier passage, « Simulation » (appliquée tout de suite) et « Lancer maintenant ».
import { Play } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import { type TaskStatus, useTasksApi } from '@/features/tasks/tasks.api'
import { useInvalidateTasks, useTasks } from '@/features/tasks/tasks.queries'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Empty } from '@/shared/ui/empty'
import { Label } from '@/shared/ui/label'
import { Skeleton } from '@/shared/ui/skeleton'
import { Switch } from '@/shared/ui/switch'

const { t, te, locale } = useI18n()
const { can } = usePermissions()
const readable = computed(() => can('system.settings.read'))
const editable = computed(() => can('system.settings.update'))
const { data: tasks, error } = useTasks()
const api = useTasksApi()
const invalidate = useInvalidateTasks()
const { handleError, message } = useApiError()
const pending = ref<string>()

/** Nom lisible d'une tâche de Kuro ; celui d'une extension reste son identifiant. */
const label = (task: TaskStatus) => {
  const key = `tasks.names.${task.name.replace('.', '_')}`
  return te(key) ? t(key) : task.name
}
const date = (value: string) => new Date(value).toLocaleString(locale.value)

async function act(key: string, task: () => Promise<unknown>) {
  pending.value = key
  try {
    await task()
    await invalidate()
  } catch (failure) {
    handleError(failure)
  } finally {
    pending.value = undefined
  }
}
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.settings.tasks')" />
    <div class="grid max-w-5xl min-w-0 gap-6 p-6">
      <p v-if="!readable" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('settings.forbidden') }}
      </p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <Skeleton v-else-if="!tasks" class="h-48" />
      <Empty v-else-if="tasks.length === 0" :title="t('tasks.empty')" />
      <template v-else>
        <Card v-for="task in tasks" :key="task.name" :data-task="task.name">
          <CardHeader>
            <CardTitle class="flex flex-wrap items-center gap-2">
              {{ label(task) }}
              <Badge v-if="task.running" variant="info">{{ t('tasks.running') }}</Badge>
              <Badge v-else-if="task.outcome === 'error'" variant="destructive">{{ t('tasks.failed') }}</Badge>
              <Badge v-else-if="task.outcome === 'ok'" variant="success">{{ t('tasks.ok') }}</Badge>
            </CardTitle>
            <CardDescription>
              {{ [task.name, t('tasks.period', { minutes: Math.max(1, Math.round(task.period_secs / 60)) })].join(' · ') }}
            </CardDescription>
          </CardHeader>
          <CardContent class="grid gap-3 text-sm">
            <p v-if="!task.finished_at" class="text-muted-foreground">{{ t('tasks.never') }}</p>
            <p v-else-if="task.outcome === 'error'" class="text-destructive">{{ task.error }}</p>
            <p v-else class="text-muted-foreground">
              {{
                [
                  t(task.last_dry_run ? 'tasks.lastSimulated' : 'tasks.lastRun', { date: date(task.finished_at) }),
                  task.duration_ms == null ? undefined : t('tasks.duration', { ms: task.duration_ms }),
                ]
                  .filter(Boolean)
                  .join(' · ')
              }}
            </p>
            <pre v-if="task.report" class="overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs">{{ JSON.stringify(task.report, null, 2) }}</pre>
            <div class="flex flex-wrap items-center gap-4 border-t pt-3">
              <div class="flex items-center gap-3">
                <Switch
                  :id="`dry-run-${task.name}`"
                  :model-value="task.dry_run"
                  :disabled="!editable || !!pending"
                  @update:model-value="(value: boolean) => act(`dry:${task.name}`, () => api.setDryRun(task.name, value))"
                />
                <Label :for="`dry-run-${task.name}`">{{ t('tasks.dryRun') }}</Label>
              </div>
              <p class="text-xs text-muted-foreground">{{ t('tasks.dryRunHelp') }}</p>
              <Button v-if="editable" variant="outline" size="sm" class="ml-auto" :disabled="!!pending || task.running" @click="act(`run:${task.name}`, () => api.run(task.name))">
                <Play />{{ t('tasks.runNow') }}
              </Button>
            </div>
          </CardContent>
        </Card>
      </template>
    </div>
  </div>
</template>
