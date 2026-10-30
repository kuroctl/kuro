<script setup lang="ts">
// Prefab Studio (ADR-0011) : brouillons de prefabs partagés entre admins, à éditer puis publier
// dans le catalogue.
import { Egg, FilePlus2, FileUp, Sparkles } from '@lucide/vue'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import ComposeImportDialog from '@/features/studio/components/ComposeImportDialog.vue'
import NewDraftDialog from '@/features/studio/components/NewDraftDialog.vue'
import { useDrafts } from '@/features/studio/studio.queries'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Skeleton } from '@/shared/ui/skeleton'

const { t, locale } = useI18n()
const { can } = usePermissions()
const { data: drafts, isPending } = useDrafts(() => can('prefabs.manage'))
const creating = ref(false)
const importing = ref(false)
const importingEgg = ref(false)

const when = (date: string) => new Date(date).toLocaleString(locale.value, { dateStyle: 'medium', timeStyle: 'short' })
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('studio.title')">
      <template v-if="can('prefabs.manage')">
        <Button variant="outline" @click="importingEgg = true"><Egg />{{ t('studio.egg.title') }}</Button>
        <Button variant="outline" @click="importing = true"><FileUp />{{ t('studio.compose.title') }}</Button>
        <Button @click="creating = true"><FilePlus2 />{{ t('studio.new.title') }}</Button>
      </template>
    </PageHeader>
    <div class="grid gap-4 p-6">
      <p class="max-w-3xl text-sm text-muted-foreground">{{ t('studio.help') }}</p>

      <p v-if="!can('prefabs.manage')" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('studio.forbidden') }}
      </p>
      <Skeleton v-else-if="isPending" class="h-32" />
      <div v-else-if="!drafts?.length" class="grid justify-items-center gap-3 rounded-xl border border-dashed bg-card px-5 py-12 text-center">
        <Sparkles class="size-8 text-primary" />
        <p class="text-sm text-muted-foreground">{{ t('studio.empty') }}</p>
        <Button variant="outline" @click="creating = true"><FilePlus2 />{{ t('studio.new.title') }}</Button>
      </div>
      <ul v-else class="grid gap-3 md:grid-cols-2">
        <li v-for="draft in drafts" :key="draft.id">
          <RouterLink
            :to="`/prefabs/studio/${draft.id}`"
            class="grid gap-2 rounded-xl border bg-card p-4 shadow-card transition-colors hover:border-primary/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            <span class="flex items-center justify-between gap-2">
              <span class="truncate font-mono text-sm font-semibold">{{ draft.prefab_id ?? t('studio.unnamed') }}</span>
              <Badge v-if="draft.prefab_version" variant="secondary" class="font-mono">{{ draft.prefab_version }}</Badge>
            </span>
            <span class="text-xs text-muted-foreground">
              {{ draft.base_id ? t('studio.basedOn', { prefab: `${draft.base_id}@${draft.base_version}` }) : t('studio.fromBlank') }}
            </span>
            <span class="text-xs text-muted-foreground">{{ t('studio.updatedAt', { date: when(draft.updated_at) }) }}</span>
          </RouterLink>
        </li>
      </ul>
    </div>
    <NewDraftDialog v-model:open="creating" />
    <ComposeImportDialog v-model:open="importing" />
    <ComposeImportDialog v-model:open="importingEgg" source="egg" />
  </div>
</template>
