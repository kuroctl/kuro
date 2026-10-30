<script setup lang="ts">
// Consentement d'un agent IA (MCP, ADR-0023) : l'admin connecté voit l'application, son propre
// compte et les droits demandés, en retient une partie, puis autorise ou refuse. Le navigateur
// repart alors chez le client, avec le code ou le refus.
import { Bot, ShieldAlert } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import { useMe } from '@/features/account/account.queries'
import { useOAuthApi } from '@/features/oauth/oauth.api'
import { useConsentRequest } from '@/features/oauth/oauth.queries'
import { usePermissionText } from '@/features/roles/permissions'
import { useApiError } from '@/shared/api/useApiError'
import { leaveTo } from '@/shared/lib/navigation'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/shared/ui/card'
import { Label } from '@/shared/ui/label'
import { Skeleton } from '@/shared/ui/skeleton'
import { Switch } from '@/shared/ui/switch'

const { t } = useI18n()
const route = useRoute()
const permission = usePermissionText()
const { handleError } = useApiError()
const { data: me } = useMe()

const id = computed(() => (typeof route.query.request === 'string' ? route.query.request : undefined))
const { data: request, error } = useConsentRequest(id)
/** Erreur signalée par l'API avant toute demande (client inconnu, URI de retour refusée). */
const REFUSALS = ['missing_client_id', 'unknown_client', 'invalid_redirect_uri']
const refused = computed(() => {
  const reason = route.query.error
  if (typeof reason === 'string') return REFUSALS.includes(reason) ? reason : 'unknown'
  return id.value ? undefined : 'missing_request'
})

/** Droits retenus : par défaut, tous ceux demandés que l'admin détient. */
const chosen = ref(new Set<string>())
watch(request, (value) => {
  chosen.value = new Set(value?.scopes.filter((scope) => scope.held).map((scope) => scope.key))
}, { immediate: true })
function choose(key: string, on: boolean) {
  const next = new Set(chosen.value)
  if (on) next.add(key)
  else next.delete(key)
  chosen.value = next
}

const api = useOAuthApi()
const pending = ref<'approve' | 'deny'>()
async function answer(kind: 'approve' | 'deny') {
  if (!id.value) return
  pending.value = kind
  try {
    leaveTo(kind === 'approve' ? await api.approve(id.value, [...chosen.value]) : await api.deny(id.value))
  } catch (failure) {
    handleError(failure)
    pending.value = undefined
  }
}

const days = computed(() => Math.round((request.value?.refresh_ttl_secs ?? 0) / 86400))
</script>

<template>
  <div class="flex min-h-dvh items-center justify-center bg-rail p-4">
    <Card class="w-full max-w-lg">
      <p v-if="refused" role="alert" class="px-6 text-sm">{{ t(`oauth.consent.refused.${refused}`) }}</p>
      <p v-else-if="error" role="alert" class="px-6 text-sm">{{ t('oauth.consent.expired') }}</p>
      <Skeleton v-else-if="!request" class="mx-6 h-64" />
      <template v-else>
        <CardHeader>
          <div class="mb-2 flex size-10 items-center justify-center rounded-lg bg-primary/12 text-primary">
            <Bot class="size-5" />
          </div>
          <CardTitle>{{ t('oauth.consent.title', { client: request.client.name }) }}</CardTitle>
          <CardDescription>{{ t('oauth.consent.help') }}</CardDescription>
        </CardHeader>
        <CardContent class="grid gap-5 text-sm">
          <dl class="grid gap-2 rounded-lg border px-4 py-3">
            <div class="flex flex-wrap items-center gap-2">
              <dt class="text-muted-foreground">{{ t('oauth.consent.client') }}</dt>
              <dd class="flex min-w-0 items-center gap-2 font-medium">
                {{ request.client.name }}
                <Badge variant="outline">{{ t('oauth.consent.unverified') }}</Badge>
              </dd>
            </div>
            <div class="flex flex-wrap gap-2">
              <dt class="text-muted-foreground">{{ t(`oauth.consent.registration.${request.client.registration}`) }}</dt>
              <dd class="min-w-0 truncate font-mono text-xs leading-5">{{ request.client.client_id }}</dd>
            </div>
            <div class="flex flex-wrap gap-2">
              <dt class="text-muted-foreground">{{ t('oauth.consent.redirect') }}</dt>
              <dd class="font-medium">{{ request.client.redirect_host }}</dd>
            </div>
            <div class="flex flex-wrap gap-2">
              <dt class="text-muted-foreground">{{ t('oauth.consent.account') }}</dt>
              <dd class="font-medium">{{ me?.user.email }}</dd>
            </div>
          </dl>

          <fieldset class="grid gap-3">
            <legend class="mb-2 font-medium">{{ t('oauth.consent.scopes') }}</legend>
            <div v-for="scope in request.scopes" :key="scope.key" class="flex items-start gap-3">
              <Switch
                :id="`scope-${scope.key}`"
                :model-value="chosen.has(scope.key)"
                :disabled="!scope.held"
                @update:model-value="(on: boolean) => choose(scope.key, on)"
              />
              <Label :for="`scope-${scope.key}`" class="grid gap-0.5 font-normal">
                <span class="flex items-center gap-2">
                  {{ permission.label(scope.key) }}
                  <ShieldAlert v-if="scope.sensitive" class="size-3.5 text-warning" :aria-label="t('oauth.consent.sensitive')" />
                </span>
                <span v-if="!scope.held" class="text-xs text-muted-foreground">{{ t('oauth.consent.notHeld') }}</span>
              </Label>
            </div>
          </fieldset>

          <p class="text-xs text-muted-foreground">{{ t('oauth.consent.duration', { days }) }}</p>
        </CardContent>
        <CardFooter class="justify-end gap-2">
          <Button variant="outline" :disabled="!!pending" @click="answer('deny')">{{ t('oauth.consent.deny') }}</Button>
          <Button :disabled="!!pending || chosen.size === 0" @click="answer('approve')">{{ t('oauth.consent.approve') }}</Button>
        </CardFooter>
      </template>
    </Card>
  </div>
</template>
