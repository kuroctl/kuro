<script setup lang="ts">
import { useI18n } from "vue-i18n"
import type { HTMLAttributes } from "vue"
import { PanelLeft } from "@lucide/vue"
import { cn } from "@/shared/lib/utils"
import { Button } from '@/shared/ui/button'
import { useSidebar } from "./utils"

const { t } = useI18n()

const props = defineProps<{
  class?: HTMLAttributes["class"]
}>()

const { toggleSidebar } = useSidebar()
</script>

<template>
  <Button
    data-sidebar="trigger"
    data-slot="sidebar-trigger"
    variant="ghost"
    size="icon"
    :class="cn('h-7 w-7', props.class)"
    @click="toggleSidebar"
  >
    <PanelLeft />
    <span class="sr-only">{{ t('nav.toggle') }}</span>
  </Button>
</template>
