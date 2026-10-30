<script setup lang="ts">
// Ports d'un nœud : ajout par plage, suppression d'un port libre.
import { Trash2 } from '@lucide/vue'
import { useQueryClient } from '@tanstack/vue-query'
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { useApiError, type FieldErrors } from '@/shared/api/useApiError'
import { Badge } from '@/shared/ui/badge'
import { Button } from '@/shared/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/shared/ui/card'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'
import { ScrollArea } from '@/shared/ui/scroll-area'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

import { useNodesApi } from '../nodes.api'
import { useAllocations } from '../nodes.queries'

const props = defineProps<{ nodeId: string }>()

const { t } = useI18n()
const client = useQueryClient()
const { addAllocations, removeAllocation } = useNodesApi()
const { handleError } = useApiError()
const { data: allocations } = useAllocations(() => props.nodeId)

const ip = ref('')
const ports = ref('')
const pending = ref(false)
const fieldErrors = ref<FieldErrors>({})

const refresh = () => client.invalidateQueries({ queryKey: ['nodes', props.nodeId, 'allocations'] })

async function add() {
  pending.value = true
  fieldErrors.value = {}
  try {
    await addAllocations(props.nodeId, { ip: ip.value || null, ports: ports.value })
    toast.success(t('nodes.allocations.added'))
    ports.value = ''
    await refresh()
  } catch (error) {
    fieldErrors.value = handleError(error, ['ip', 'ports'])
  } finally {
    pending.value = false
  }
}

async function remove(allocationId: string) {
  try {
    await removeAllocation(props.nodeId, allocationId)
    await refresh()
  } catch (error) {
    handleError(error)
  }
}
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ t('nodes.allocations.title') }}</CardTitle>
      <CardDescription>{{ t('nodes.allocations.help') }}</CardDescription>
    </CardHeader>
    <CardContent class="grid gap-6">
      <form class="grid items-end gap-3 sm:grid-cols-[1fr_2fr_auto]" @submit.prevent="add">
        <div class="grid gap-2">
          <Label for="allocation-ip">{{ t('nodes.allocations.ip') }}</Label>
          <Input id="allocation-ip" v-model="ip" placeholder="0.0.0.0" />
          <p v-if="fieldErrors.ip" class="text-sm text-destructive">{{ fieldErrors.ip }}</p>
        </div>
        <div class="grid gap-2">
          <Label for="allocation-ports">{{ t('nodes.allocations.ports') }}</Label>
          <Input id="allocation-ports" v-model="ports" placeholder="25565-25600" required />
          <p v-if="fieldErrors.ports" class="text-sm text-destructive">{{ fieldErrors.ports }}</p>
        </div>
        <Button type="submit" :disabled="pending || !ports">{{ t('nodes.allocations.add') }}</Button>
      </form>

      <p v-if="!allocations?.length" class="text-sm text-muted-foreground">{{ t('nodes.allocations.empty') }}</p>
      <ScrollArea v-else class="max-h-96">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{{ t('nodes.allocations.address') }}</TableHead>
              <TableHead>{{ t('nodes.allocations.server') }}</TableHead>
              <TableHead class="w-12" />
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="allocation in allocations" :key="allocation.id">
              <TableCell class="font-mono text-sm">{{ `${allocation.ip}:${allocation.port}` }}</TableCell>
              <TableCell>
                <RouterLink v-if="allocation.server_id" :to="`/servers/${allocation.server_id}`" class="hover:underline">
                  {{ allocation.server_name }}
                </RouterLink>
                <Badge v-else variant="outline">{{ t('nodes.allocations.free') }}</Badge>
              </TableCell>
              <TableCell>
                <Button
                  v-if="!allocation.server_id"
                  size="icon"
                  variant="ghost"
                  :aria-label="t('nodes.allocations.remove')"
                  @click="remove(allocation.id)"
                >
                  <Trash2 class="size-4" />
                </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </ScrollArea>
    </CardContent>
  </Card>
</template>
