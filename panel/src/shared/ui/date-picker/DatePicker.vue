<script setup lang="ts">
// Sélecteur de date du design system : un champ qui ouvre le calendrier de shadcn-vue. La valeur
// reste une chaîne `AAAA-MM-JJ` (vide : aucune date), comme celle d'un `<input type="date">`.
import { type DateValue, getLocalTimeZone, parseDate, today } from "@internationalized/date"
import { CalendarDays } from "@lucide/vue"
import type { HTMLAttributes } from "vue"
import { computed, ref } from "vue"
import { useI18n } from "vue-i18n"

import { cn } from "@/shared/lib/utils"
import { Button } from "@/shared/ui/button"
import { Calendar } from "@/shared/ui/calendar"
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover"

const props = defineProps<{ id?: string, placeholder?: string, class?: HTMLAttributes["class"] }>()
const model = defineModel<string>({ default: "" })
const { t, locale } = useI18n()
const open = ref(false)

const value = computed<DateValue | undefined>({
  get: () => (model.value ? parseDate(model.value) : undefined),
  set: (date) => {
    model.value = date ? date.toString() : ""
    open.value = false
  },
})

const label = computed(() =>
  model.value ? new Intl.DateTimeFormat(locale.value, { dateStyle: "short" }).format(new Date(`${model.value}T00:00:00`)) : "",
)
</script>

<template>
  <Popover v-model:open="open">
    <PopoverTrigger as-child>
      <button
        :id="props.id"
        type="button"
        data-slot="date-picker"
        :class="cn(
          'border-input flex h-10 w-full min-w-0 items-center gap-2 rounded-lg border bg-muted px-3 text-left text-sm transition-[color,border-color,box-shadow] outline-none hover:border-[color-mix(in_oklch,var(--input)_50%,var(--foreground))] focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/25 data-[state=open]:border-ring data-[state=open]:ring-3 data-[state=open]:ring-ring/25',
          props.class,
        )"
      >
        <span :class="cn('min-w-0 flex-1 truncate', !label && 'text-muted-foreground')">{{ label || props.placeholder || t('common.pickDate') }}</span>
        <CalendarDays class="size-4 shrink-0 text-muted-foreground" />
      </button>
    </PopoverTrigger>
    <PopoverContent class="w-auto p-0" align="start">
      <Calendar v-model="value" :locale="locale" layout="month-and-year" initial-focus />
      <div class="flex justify-between border-t px-3 py-2">
        <Button variant="ghost" size="sm" @click="value = undefined">{{ t('common.clear') }}</Button>
        <Button variant="ghost" size="sm" class="text-primary" @click="value = today(getLocalTimeZone())">{{ t('common.today') }}</Button>
      </div>
    </PopoverContent>
  </Popover>
</template>
