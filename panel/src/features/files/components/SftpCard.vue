<script setup lang="ts">
// Connexion SFTP au serveur (ADR-0047) : hôte, port et nom d'utilisateur propres au compte
// connecté, empreinte de la clé d'hôte à comparer à celle que montre le client. L'accès se fait
// par une clé SSH du compte.
import { Copy } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'
import { toast } from 'vue-sonner'

import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Skeleton } from '@/shared/ui/skeleton'

import { useSftpConnection } from '../files.queries'

const props = defineProps<{ server: string }>()
const { t } = useI18n()
const { data: connection, isPending } = useSftpConnection(
  () => props.server,
  () => true,
)

const address = computed(() => {
  const value = connection.value
  if (!value?.host || !value.port) return undefined
  return `sftp://${value.username}@${value.host}:${value.port}`
})

const rows = computed(() => {
  const value = connection.value
  if (!value) return []
  return [
    // Sans DNS ni adresse saisie, la ligne le dit : le reste reste utile.
    { key: 'host', label: t('files.sftp.host'), value: value.host ?? undefined },
    { key: 'port', label: t('files.sftp.port'), value: value.port ? String(value.port) : '' },
    { key: 'username', label: t('files.sftp.username'), value: value.username },
  ]
})

async function copy(value: string) {
  await navigator.clipboard.writeText(value)
  toast.success(t('files.sftp.copied'))
}
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ t('files.sftp.title') }}</CardTitle>
      <CardDescription>{{ t('files.sftp.help') }}</CardDescription>
    </CardHeader>
    <CardContent class="grid gap-4 text-sm">
      <Skeleton v-if="isPending" class="h-24" />
      <template v-else-if="connection">
        <p v-if="!connection.enabled" class="rounded-lg border border-warning/35 bg-warning/8 px-4 py-3">
          {{ t('files.sftp.disabled') }}
        </p>
        <p v-else-if="!connection.available" class="rounded-lg border border-warning/35 bg-warning/8 px-4 py-3">
          {{ t('files.sftp.unavailable', { node: connection.node_name }) }}
        </p>
        <template v-else>
          <dl class="grid gap-2 sm:grid-cols-[8rem_minmax(0,1fr)]">
            <template v-for="row in rows" :key="row.key">
              <dt class="text-muted-foreground">{{ row.label }}</dt>
              <dd v-if="row.value === undefined" class="text-warning">
                {{ t('files.sftp.noAddress', { node: connection.node_name }) }}
              </dd>
              <dd v-else class="flex min-w-0 items-center gap-2">
                <code class="min-w-0 truncate font-mono text-xs">{{ row.value }}</code>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  class="size-7"
                  :aria-label="t('files.sftp.copy', { field: row.label })"
                  @click="copy(row.value)"
                >
                  <Copy class="size-3.5" />
                </Button>
              </dd>
            </template>
            <dt class="text-muted-foreground">{{ t('files.sftp.hostKey') }}</dt>
            <dd class="min-w-0 truncate font-mono text-xs">{{ connection.host_key_fingerprint }}</dd>
          </dl>
          <a v-if="address" :href="address" class="justify-self-start text-xs text-primary underline-offset-4 hover:underline">
            {{ t('files.sftp.open') }}
          </a>
          <p class="text-xs text-muted-foreground">
            {{ t('files.sftp.keys') }}
            <RouterLink to="/account/security" class="text-primary underline-offset-4 hover:underline">
              {{ t('files.sftp.keysLink') }}
            </RouterLink>
          </p>
        </template>
      </template>
    </CardContent>
  </Card>
</template>
