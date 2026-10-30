<script setup lang="ts">
// Sources de contenu (ADR-0029, ADR-0030) : Modrinth, ouverte, puis une carte par source qui
// demande la clé d'API de l'installation (CurseForge, Steam Workshop), et SteamGridDB pour les
// images du Studio (ADR-0040).
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { usePermissions } from '@/features/account/account.queries'
import SourceKeyCard from '@/features/sources/components/SourceKeyCard.vue'
import { useContentSources } from '@/features/sources/sources.queries'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Card, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Skeleton } from '@/shared/ui/skeleton'

/** Où créer la clé de chaque source ; une source inconnue du panel n'est pas affichée. */
const CONSOLES: Record<string, string> = {
  curseforge: 'https://console.curseforge.com/',
  'steam-workshop': 'https://steamcommunity.com/dev/apikey',
  steamgriddb: 'https://www.steamgriddb.com/profile/preferences/api',
}

const { t } = useI18n()
const { can } = usePermissions()
const allowed = computed(() => can('system.content_sources.manage'))
const { data: sources, error } = useContentSources(allowed)
const { message } = useApiError()
const keyed = computed(() => (sources.value ?? []).filter((source) => source.provider in CONSOLES))
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="t('subnav.settings.sources')" />
    <div class="grid max-w-4xl min-w-0 gap-6 p-6">
      <p v-if="!allowed" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground shadow-card">
        {{ t('sources.forbidden') }}
      </p>
      <p v-else-if="error" role="alert" class="rounded-lg border border-destructive/35 bg-destructive/8 px-4 py-3.5 text-sm">{{ message(error) }}</p>
      <Skeleton v-else-if="!sources" class="h-60" />
      <template v-else>
        <Card>
          <CardHeader>
            <CardTitle class="flex items-center gap-2">
              {{ t('content.sources.modrinth') }}
              <Badge variant="success">{{ t('sources.active') }}</Badge>
            </CardTitle>
            <CardDescription>{{ t('sources.help.modrinth') }}</CardDescription>
          </CardHeader>
        </Card>
        <SourceKeyCard v-for="source in keyed" :key="source.provider" :source="source" :console-url="CONSOLES[source.provider]!" />
      </template>
    </div>
  </div>
</template>
