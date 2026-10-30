<script setup lang="ts">
// Applications connectées (ADR-0023) : agents IA à qui l'admin a délégué des droits, et
// révocation. Une révocation coupe aussitôt tous les jetons de l'accès.
import { Bot } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { type Grant, useOAuthApi } from '@/features/oauth/oauth.api'
import { useGrants, useInvalidateGrants } from '@/features/oauth/oauth.queries'
import { usePermissionText } from '@/features/roles/permissions'
import { useApiError } from '@/shared/api/useApiError'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'
import { Skeleton } from '@/shared/ui/skeleton'

const { t, locale } = useI18n()
const permission = usePermissionText()
const { data: grants, error, isPending } = useGrants()
const invalidate = useInvalidateGrants()
const { handleError } = useApiError()
const api = useOAuthApi()
const revoking = ref<string>()
const dates = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }))

async function revoke(grant: Grant) {
  revoking.value = grant.id
  try {
    await api.revokeGrant(grant.id)
    await invalidate()
    toast.success(t('account.applications.revoked', { client: grant.client_name }))
  } catch (failure) {
    handleError(failure)
  } finally {
    revoking.value = undefined
  }
}
</script>

<template>
  <div>
    <PageHeader :title="t('subnav.account.applications')" />
    <div class="grid max-w-3xl gap-4 p-6">
      <p class="text-sm text-muted-foreground">{{ t('account.applications.help') }}</p>
      <Skeleton v-if="isPending" class="h-32" />
      <p v-else-if="error" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground">
        {{ t('account.applications.unavailable') }}
      </p>
      <p v-else-if="!grants?.length" class="rounded-xl border bg-card px-5 py-8 text-center text-sm text-muted-foreground">
        {{ t('account.applications.empty') }}
      </p>
      <Card v-for="grant in grants" v-else :key="grant.id">
        <CardHeader class="flex flex-row items-start gap-3">
          <div class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/12 text-primary">
            <Bot class="size-4" />
          </div>
          <div class="grid min-w-0 flex-1 gap-1">
            <CardTitle class="flex items-center gap-2">
              {{ grant.client_name }}
              <Badge variant="outline">{{ t(`oauth.consent.registration.${grant.registration}`) }}</Badge>
            </CardTitle>
            <p class="truncate font-mono text-xs text-muted-foreground">{{ grant.client_id }}</p>
          </div>
          <Button variant="outline" size="sm" :disabled="revoking === grant.id" @click="revoke(grant)">
            {{ t('account.applications.revoke') }}
          </Button>
        </CardHeader>
        <CardContent class="grid gap-3 text-sm">
          <ul class="flex flex-wrap gap-1.5">
            <li v-for="scope in grant.scopes" :key="scope">
              <Badge variant="secondary">{{ permission.label(scope) }}</Badge>
            </li>
          </ul>
          <p class="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
            <span>{{ t('account.applications.granted', { date: dates.format(new Date(grant.created_at)) }) }}</span>
            <span>{{
              grant.last_used_at
                ? t('account.applications.used', { date: dates.format(new Date(grant.last_used_at)) })
                : t('account.applications.neverUsed')
            }}</span>
          </p>
        </CardContent>
      </Card>
    </div>
  </div>
</template>
