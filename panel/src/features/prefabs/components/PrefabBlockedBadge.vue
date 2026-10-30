<script setup lang="ts">
// Version qui ne se déploie pas partout, avec la raison au survol (ADR-0011) :
// - « Pas encore déployable » : minimum de ressources au-dessus d'un plafond du projet (ADR-0031)
//   ou compte Steam de l'installation à régler (ADR-0033) ;
// - « Brouillon · nœud de test » : version marquée `x-status`, déployable sur un nœud de test
//   seulement, pour la mettre au point.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Schemas } from '@/shared/api/types'
import { Badge } from '@/shared/ui/badge'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/shared/ui/tooltip'

const props = defineProps<{
  prefab: Pick<Schemas['PrefabSummaryDto'], 'deployable' | 'test_nodes_only' | 'draft_reason' | 'over_ceiling' | 'steam_account_missing'>
}>()

const { t } = useI18n()
const label = computed(() => (props.prefab.deployable ? t('prefabs.catalog.testNodesOnly') : t('prefabs.catalog.draft')))
const reason = computed(() => {
  if (props.prefab.over_ceiling.length) {
    const resources = props.prefab.over_ceiling.map((resource) => t(`servers.fields.${resource}`)).join(', ')
    return t('prefabs.catalog.overCeiling', { resources })
  }
  if (props.prefab.steam_account_missing) return t('prefabs.catalog.steamAccountMissing')
  if (props.prefab.draft_reason) return t('prefabs.catalog.draftReason', { reason: props.prefab.draft_reason })
  return t('prefabs.catalog.draftNoReason')
})
</script>

<template>
  <Tooltip v-if="!prefab.deployable || prefab.test_nodes_only">
    <TooltipTrigger as-child>
      <Badge variant="warning" tabindex="0" :aria-label="`${label} : ${reason}`">{{ label }}</Badge>
    </TooltipTrigger>
    <TooltipContent class="max-w-xs">{{ reason }}</TooltipContent>
  </Tooltip>
</template>
