<script setup lang="ts">
// Détail d'un nœud : enrôlement (en attente ou révoqué), emplacement (région, lieu, position sur la carte), adresse publique et nom
// DNS, essais du Studio, ports, et suppression tant qu'il n'est pas en ligne.
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'

import { usePermissions } from '@/features/account/account.queries'
import NodeAddressCard from '@/features/dns/components/NodeAddressCard.vue'
import AllocationsCard from '@/features/nodes/components/AllocationsCard.vue'
import NodeDeleteCard from '@/features/nodes/components/NodeDeleteCard.vue'
import NodeEnrollmentCard from '@/features/nodes/components/NodeEnrollmentCard.vue'
import NodeLocationCard from '@/features/nodes/components/NodeLocationCard.vue'
import NodeTrialsCard from '@/features/nodes/components/NodeTrialsCard.vue'
import { useNode } from '@/features/nodes/nodes.queries'
import { nodeBadge } from '@/features/nodes/status'
import PageHeader from '@/shared/layout/PageHeader.vue'
import { Badge } from '@/shared/ui/badge'

const { t } = useI18n()
const route = useRoute('/servers/nodes/[id]')
const { data: node } = useNode(() => route.params.id)
const { can } = usePermissions()
</script>

<template>
  <div class="flex flex-1 flex-col">
    <PageHeader :title="node?.display_name ?? node?.name ?? ''">
      <Badge v-if="node" :variant="nodeBadge[node.status]">{{ t(`nodes.status.${node.status}`) }}</Badge>
    </PageHeader>
    <div class="grid max-w-4xl gap-6 p-6">
      <NodeEnrollmentCard
        v-if="node && can('nodes.enrollment') && (node.status === 'pending' || node.status === 'revoked')"
        :key="node.id"
        :node="node"
      />
      <NodeLocationCard v-if="node" :node="node" :editable="can('nodes.update')" />
      <NodeAddressCard v-if="node" :node="node" :editable="can('nodes.update')" />
      <NodeTrialsCard v-if="node" :node="node" :editable="can('nodes.update')" />
      <AllocationsCard :node-id="route.params.id" />
      <NodeDeleteCard v-if="node && can('nodes.delete') && node.status !== 'online'" :node="node" />
    </div>
  </div>
</template>
