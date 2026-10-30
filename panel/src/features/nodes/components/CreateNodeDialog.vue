<script setup lang="ts">
// Ajout d'un nœud : nom technique et libellé, puis le ticket d'enrôlement, montré une seule fois.
// L'emplacement (région, lieu) se règle ensuite sur la fiche du nœud.
import { Plus } from '@lucide/vue'
import { reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { type FieldErrors, useApiError } from '@/shared/api/useApiError'
import { Button } from '@/shared/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from '@/shared/ui/dialog'
import { Input } from '@/shared/ui/input'
import { Label } from '@/shared/ui/label'

import { type NodeTicket, useNodesApi } from '../nodes.api'
import { useInvalidateNodes } from '../nodes.queries'
import EnrollmentTicket from './EnrollmentTicket.vue'

const { t } = useI18n()
const router = useRouter()
const api = useNodesApi()
const { handleError } = useApiError()
const invalidate = useInvalidateNodes()

const open = ref(false)
const pending = ref(false)
const errors = ref<FieldErrors>({})
const form = reactive({ name: '', display_name: '' })
const ticket = ref<NodeTicket>()

function reset(isOpen: boolean) {
  open.value = isOpen
  if (!isOpen) {
    Object.assign(form, { name: '', display_name: '' })
    errors.value = {}
    ticket.value = undefined
  }
}

async function submit() {
  pending.value = true
  errors.value = {}
  try {
    ticket.value = await api.create({ name: form.name.trim(), display_name: form.display_name.trim() || null })
    await invalidate()
  } catch (error) {
    errors.value = handleError(error, ['name', 'display_name'])
  } finally {
    pending.value = false
  }
}

async function openNode() {
  const id = ticket.value?.node.id
  reset(false)
  if (id) await router.push(`/servers/nodes/${id}`)
}
</script>

<template>
  <Dialog :open="open" @update:open="reset">
    <DialogTrigger as-child>
      <Button size="sm"><Plus />{{ t('nodes.create.action') }}</Button>
    </DialogTrigger>
    <DialogContent class="max-h-[90dvh] overflow-y-auto sm:max-w-2xl">
      <DialogHeader>
        <DialogTitle>{{ t('nodes.create.title') }}</DialogTitle>
        <DialogDescription>{{ ticket ? t('nodes.create.ticketHelp') : t('nodes.create.help') }}</DialogDescription>
      </DialogHeader>
      <template v-if="ticket">
        <EnrollmentTicket :ticket="ticket.enrollment" />
        <DialogFooter>
          <Button type="button" variant="ghost" @click="reset(false)">{{ t('common.close') }}</Button>
          <Button type="button" @click="openNode">{{ t('nodes.create.open') }}</Button>
        </DialogFooter>
      </template>
      <form v-else class="grid gap-4" @submit.prevent="submit">
        <div class="grid gap-1.5">
          <Label for="node-name">{{ t('nodes.create.name') }}</Label>
          <Input id="node-name" v-model="form.name" autocomplete="off" :placeholder="t('nodes.create.namePlaceholder')" :aria-invalid="!!errors.name" />
          <p v-if="errors.name" class="text-sm text-destructive">{{ errors.name }}</p>
          <p v-else class="text-xs text-muted-foreground">{{ t('nodes.create.nameHelp') }}</p>
        </div>
        <div class="grid gap-1.5">
          <Label for="node-label">{{ t('nodes.create.label') }}</Label>
          <Input id="node-label" v-model="form.display_name" autocomplete="off" :aria-invalid="!!errors.display_name" />
          <p v-if="errors.display_name" class="text-sm text-destructive">{{ errors.display_name }}</p>
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" @click="reset(false)">{{ t('common.cancel') }}</Button>
          <Button type="submit" :disabled="pending || !form.name.trim()">{{ t('nodes.create.submit') }}</Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
</template>
