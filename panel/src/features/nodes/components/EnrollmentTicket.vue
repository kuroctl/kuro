<script setup lang="ts">
// Ticket d'enrôlement, montré une seule fois : le jeton se copie à part et se colle à l'invite
// de `read`, jamais dans la commande (il finirait dans l'historique du shell). L'adresse de
// l'API est celle du panel : le certificat des daemons porte le même nom.
import { Copy } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { toast } from 'vue-sonner'

import { Button } from '@/shared/ui/button'
import { Label } from '@/shared/ui/label'

import type { NodeTicket } from '../nodes.api'

const INSTALLER = 'https://get.kuroctl.com/node'

const props = defineProps<{ ticket: NodeTicket['enrollment'] }>()
const { t } = useI18n()

const fingerprint = computed(() => `sha256:${props.ticket.ca_fingerprint}`)
const minutes = computed(() => Math.max(1, Math.round(props.ticket.expires_in / 60)))
const command = computed(() =>
  [
    `curl -fsSLo install.sh ${INSTALLER}`,
    'read -rs KURO_ENROLL_TOKEN && export KURO_ENROLL_TOKEN',
    `sudo -E bash install.sh --api https://${window.location.hostname}:${props.ticket.daemon_port} --ca-fingerprint ${fingerprint.value}`,
  ].join('\n'),
)

async function copy(text: string, done: string) {
  await navigator.clipboard.writeText(text)
  toast.success(done)
}
</script>

<template>
  <div class="grid gap-4">
    <p class="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-sm">{{ t('nodes.enroll.once', { minutes }) }}</p>
    <div class="grid gap-1.5">
      <Label for="enroll-token">{{ t('nodes.enroll.token') }}</Label>
      <div class="flex gap-2">
        <code id="enroll-token" data-testid="enroll-token" class="min-w-0 flex-1 truncate rounded-md border bg-muted px-3 py-2 font-mono text-xs">{{ ticket.token }}</code>
        <Button variant="outline" size="icon" :aria-label="t('nodes.enroll.copyToken')" @click="copy(ticket.token, t('nodes.enroll.tokenCopied'))">
          <Copy />
        </Button>
      </div>
    </div>
    <div class="grid gap-1.5">
      <p class="text-sm font-medium">{{ t('nodes.enroll.fingerprint') }}</p>
      <code class="break-all rounded-md border bg-muted px-3 py-2 font-mono text-xs">{{ fingerprint }}</code>
      <p class="text-xs text-muted-foreground">{{ t('nodes.enroll.fingerprintHelp') }}</p>
    </div>
    <div class="grid gap-1.5">
      <div class="flex items-center justify-between gap-2">
        <p class="text-sm font-medium">{{ t('nodes.enroll.command') }}</p>
        <Button variant="ghost" size="sm" @click="copy(command, t('nodes.enroll.commandCopied'))"><Copy />{{ t('nodes.enroll.copy') }}</Button>
      </div>
      <pre data-testid="enroll-command" class="overflow-x-auto rounded-md border bg-muted px-3 py-2 font-mono text-xs">{{ command }}</pre>
      <p class="text-xs text-muted-foreground">{{ t('nodes.enroll.commandHelp') }}</p>
    </div>
  </div>
</template>
