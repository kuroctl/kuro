<script setup lang="ts">
// Adresses d'un serveur (ADR-0026) : avec le DNS configuré et un nom, `<label>.<base>:<port>`
// par port public (l'IP reste indiquée dessous), sinon `IP:port`. Un admin avec
// `servers.update` choisit le nom ; vide, le serveur n'a plus de nom.
import { Copy, Pencil } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { usePermissions } from '@/features/account/account.queries'
import type { Server } from '@/features/servers/servers.api'
import { useInvalidateServers } from '@/features/servers/servers.queries'
import { useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/shared/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { useDnsApi } from '../dns.api'
import { useDnsZone, useInvalidateDns } from '../dns.queries'
import { serverHost } from '../hostnames'

const props = defineProps<{ server: Server }>()
const { t } = useI18n()
const { can } = usePermissions()
const { data: zone } = useDnsZone()
const api = useDnsApi()
const invalidateServers = useInvalidateServers()
const invalidateDns = useInvalidateDns()
const { handleError } = useApiError()

const base = computed(() => zone.value?.base_domain ?? null)
const host = computed(() => serverHost(zone.value, props.server.dns_label))
const editable = computed(() => !!base.value && can('servers.update'))

const open = ref(false)
const label = ref('')
const error = ref<string>()
const pending = ref(false)

watch(open, (opened) => {
  if (!opened) return
  label.value = props.server.dns_label ?? ''
  error.value = undefined
})

async function save() {
  pending.value = true
  error.value = undefined
  try {
    const value = label.value.trim().toLowerCase()
    await api.setServerLabel(props.server.id, value || null)
    open.value = false
    toast.success(t(value ? 'dns.server.saved' : 'dns.server.cleared'))
    await Promise.all([invalidateServers(), invalidateDns()])
  } catch (failure) {
    error.value = handleError(failure, ['label']).label
  } finally {
    pending.value = false
  }
}

async function copy(text: string) {
  await navigator.clipboard.writeText(text)
  toast.success(t('dns.copied'))
}
</script>

<template>
  <Card>
    <CardHeader class="flex flex-row items-center justify-between gap-2">
      <CardTitle>{{ t('servers.overview.addresses') }}</CardTitle>
      <Button v-if="editable" variant="ghost" size="sm" class="h-7" @click="open = true">
        <Pencil />{{ t(server.dns_label ? 'dns.server.edit' : 'dns.server.add') }}
      </Button>
    </CardHeader>
    <CardContent class="grid gap-2 font-mono text-sm">
      <div v-for="allocation in server.allocations" :key="allocation.name" class="grid gap-0.5">
        <div class="flex min-w-0 items-center gap-1">
          <span class="truncate">{{ `${host ?? allocation.ip}:${allocation.port}` }}</span>
          <span class="shrink-0 text-xs text-muted-foreground">{{ t('servers.overview.port', { name: allocation.name }) }}</span>
          <Button
            v-if="host"
            variant="ghost"
            size="icon"
            class="ml-auto size-7 shrink-0"
            :aria-label="t('dns.server.copy', { address: `${host}:${allocation.port}` })"
            @click="copy(`${host}:${allocation.port}`)"
          >
            <Copy class="size-3.5" />
          </Button>
        </div>
        <span v-if="host" class="text-xs text-muted-foreground">{{ `${allocation.ip}:${allocation.port}` }}</span>
      </div>
    </CardContent>
  </Card>

  <Dialog v-model:open="open">
    <DialogContent class="sm:max-w-md">
      <DialogHeader>
        <DialogTitle>{{ t('dns.server.title') }}</DialogTitle>
        <DialogDescription>{{ t('dns.server.help') }}</DialogDescription>
      </DialogHeader>
      <form class="grid gap-2" @submit.prevent="save">
        <Label for="server-dns-label">{{ t('dns.server.label') }}</Label>
        <div class="flex items-center gap-2">
          <Input
            id="server-dns-label"
            v-model="label"
            class="font-mono"
            autocomplete="off"
            spellcheck="false"
            :aria-invalid="error ? true : undefined"
          />
          <span class="shrink-0 font-mono text-sm text-muted-foreground">{{ `.${base}` }}</span>
        </div>
        <p v-if="error" class="text-xs text-destructive">{{ error }}</p>
        <p v-else class="text-xs text-muted-foreground">{{ t('dns.server.labelHelp') }}</p>
        <DialogFooter class="mt-2">
          <Button type="button" variant="ghost" @click="open = false">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="pending">{{ t('common.save') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
