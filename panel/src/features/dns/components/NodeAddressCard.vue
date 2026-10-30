<script setup lang="ts">
// Adresse publique d'un nœud (ADR-0026) : IPv4 et IPv6 facultative, publiées sous le nom du nœud
// quand le DNS est configuré. Indispensable derrière un NAT. Un champ vide retire l'adresse.
// Bouton propre à la carte : la barre d'enregistrement de la page sert déjà à l'emplacement.
import { Copy } from '@lucide/vue'
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import type { Node } from '@/features/nodes/nodes.api'
import { useInvalidateNodes } from '@/features/nodes/nodes.queries'
import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { useDnsApi } from '../dns.api'
import { useDnsZone, useInvalidateDns } from '../dns.queries'
import { nodeHost } from '../hostnames'

const props = defineProps<{ node: Node; editable: boolean }>()
const { t } = useI18n()
const api = useDnsApi()
const { data: zone } = useDnsZone()
const invalidateNodes = useInvalidateNodes()
const invalidateDns = useInvalidateDns()
const { handleError } = useApiError()

const form = reactive({ ipv4: '', ipv6: '' })
const errors = ref<FieldErrors>({})
const pending = ref(false)

function load(node: Node) {
  form.ipv4 = node.public_ipv4 ?? ''
  form.ipv6 = node.public_ipv6 ?? ''
}
watch(() => props.node, load, { immediate: true })

const text = (value: string) => (value.trim() === '' ? null : value.trim())
const dirty = computed(
  () => text(form.ipv4) !== (props.node.public_ipv4 ?? null) || text(form.ipv6) !== (props.node.public_ipv6 ?? null),
)
const host = computed(() => nodeHost(zone.value, props.node.name))

async function save() {
  pending.value = true
  errors.value = {}
  try {
    await api.setNodeAddress(props.node.id, { ipv4: text(form.ipv4), ipv6: text(form.ipv6) })
    toast.success(t('dns.node.saved'))
    await Promise.all([invalidateNodes(), invalidateDns()])
  } catch (error) {
    errors.value = handleError(error, ['address', 'ipv4', 'ipv6'])
  } finally {
    pending.value = false
  }
}

async function copy() {
  if (!host.value) return
  await navigator.clipboard.writeText(host.value)
  toast.success(t('dns.copied'))
}
</script>

<template>
  <form @submit.prevent="save">
    <Card>
      <CardHeader>
        <CardTitle>{{ t('dns.node.title') }}</CardTitle>
        <CardDescription>{{ t('dns.node.help') }}</CardDescription>
      </CardHeader>
      <CardContent class="grid gap-4">
        <div class="grid gap-4 sm:grid-cols-2">
          <div class="grid content-start gap-1.5">
            <Label for="node-ipv4">{{ t('dns.node.ipv4') }}</Label>
            <Input
              id="node-ipv4"
              v-model="form.ipv4"
              placeholder="203.0.113.10"
              autocomplete="off"
              spellcheck="false"
              class="font-mono"
              :disabled="!editable"
              :aria-invalid="!!(errors.ipv4 || errors.address)"
            />
            <p v-if="errors.ipv4" class="text-xs text-destructive">{{ errors.ipv4 }}</p>
          </div>
          <div class="grid content-start gap-1.5">
            <Label for="node-ipv6">{{ t('dns.node.ipv6') }}</Label>
            <Input
              id="node-ipv6"
              v-model="form.ipv6"
              placeholder="2001:db8::10"
              autocomplete="off"
              spellcheck="false"
              class="font-mono"
              :disabled="!editable"
              :aria-invalid="!!(errors.ipv6 || errors.address)"
            />
            <p v-if="errors.ipv6" class="text-xs text-destructive">{{ errors.ipv6 }}</p>
          </div>
        </div>
        <p v-if="errors.address" class="text-xs text-destructive">{{ errors.address }}</p>
        <div v-if="editable" class="flex gap-2">
          <Button type="submit" :disabled="pending || !dirty">{{ t('dns.node.save') }}</Button>
          <Button v-if="dirty" type="button" variant="ghost" :disabled="pending" @click="load(node)">{{ t('common.reset') }}</Button>
        </div>
        <div v-if="host" class="grid gap-1.5">
          <p class="text-xs text-muted-foreground">{{ t('dns.node.host') }}</p>
          <div class="flex items-center gap-2">
            <code class="min-w-0 flex-1 truncate rounded-md border bg-muted px-3 py-2 font-mono text-xs">{{ host }}</code>
            <Button type="button" variant="outline" size="icon" :aria-label="t('dns.copy')" @click="copy">
              <Copy class="size-4" />
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  </form>
</template>
