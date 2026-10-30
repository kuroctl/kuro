<script setup lang="ts">
// Courbe d'historique sur les charts shadcn-vue (Unovis) : une ou deux séries horodatées, de 0
// à `max` (ou à un plafond rond au-dessus du plus haut point), avec repères de temps, valeurs sur
// l'axe, info-bulle au survol et la valeur du dernier point en légende.
import { VisArea, VisAxis, VisLine, VisScatter, VisXYContainer } from '@unovis/vue'
import { computed, useId } from 'vue'
import { useI18n } from 'vue-i18n'

import { type ChartConfig, ChartContainer, ChartCrosshair, ChartTooltip, ChartTooltipContent, componentToString } from '@/shared/ui/chart'

const props = defineProps<{
  /** Points dans l'ordre du temps. */
  points: { at: string; values: number[] }[]
  /** Libellé de chaque série. */
  labels: string[]
  /** Formatage d'une valeur pour l'axe, la légende et l'info-bulle. */
  format: (value: number) => string
  max?: number
  /** Plage affichée, pour le format des repères de temps. */
  range: '24h' | '7d' | '90d'
}>()

/** Une ligne du graphe : l'instant, puis la première et la seconde série. */
type Row = { at: number; a: number; b: number }

const { locale } = useI18n()
const uid = useId()

const rows = computed<Row[]>(() => props.points.map((p) => ({ at: new Date(p.at).getTime(), a: p.values[0] ?? 0, b: p.values[1] ?? 0 })))

/** Plafond rond (1, 2, 5 × 10ⁿ) au-dessus d'une valeur, pour une échelle lisible. */
function niceCeil(value: number): number {
  if (value <= 0) return 1
  const magnitude = 10 ** Math.floor(Math.log10(value))
  const step = [1, 2, 5, 10].find((s) => s * magnitude >= value) ?? 10
  return step * magnitude
}

const top = computed(() => props.max ?? niceCeil(Math.max(0, ...rows.value.flatMap((r) => [r.a, r.b]))))

const config = computed(
  () =>
    ({
      a: { label: props.labels[0] ?? '', color: 'var(--chart-1)' },
      b: { label: props.labels[1] ?? '', color: 'var(--chart-2)' },
    }) satisfies ChartConfig,
)
const colors = computed(() => [config.value.a.color, config.value.b.color])

const x = (d: Row) => d.at
const yA = (d: Row) => d.a
const yB = (d: Row) => d.b

const svgDefs = `
  <linearGradient id="fill-a-${uid}" x1="0" y1="0" x2="0" y2="1">
    <stop offset="5%" stop-color="var(--chart-1)" stop-opacity="0.8" />
    <stop offset="95%" stop-color="var(--chart-1)" stop-opacity="0.1" />
  </linearGradient>
  <linearGradient id="fill-b-${uid}" x1="0" y1="0" x2="0" y2="1">
    <stop offset="5%" stop-color="var(--chart-2)" stop-opacity="0.5" />
    <stop offset="95%" stop-color="var(--chart-2)" stop-opacity="0.05" />
  </linearGradient>
`

const timeFormat = computed(() => new Intl.DateTimeFormat(locale.value, props.range === '24h' ? { hour: '2-digit', minute: '2-digit' } : { day: 'numeric', month: 'short' }))
const tooltipFormat = computed(() => new Intl.DateTimeFormat(locale.value, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }))
const tickX = (value: number) => timeFormat.value.format(new Date(value))

// Rendu une fois au montage (le gabarit est mis en cache par point) ; les formats lisent la
// langue au moment du survol.
const tooltip = componentToString(config.value, ChartTooltipContent, {
  labelFormatter: (d: number | Date) => tooltipFormat.value.format(new Date(d)),
  valueFormatter: (value: unknown) => props.format(Number(value)),
})

const latest = computed(() => props.points.at(-1)?.values ?? [])
</script>

<template>
  <div class="grid gap-1">
    <div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
      <span v-for="(label, index) in labels" :key="label" class="flex items-center gap-1.5">
        <span class="inline-block size-2 rounded-xs" :style="{ backgroundColor: colors[index] }" aria-hidden="true" />
        {{ label }}
        <span v-if="latest[index] !== undefined" class="font-semibold tabular-nums text-foreground">{{ format(latest[index]!) }}</span>
      </span>
    </div>
    <ChartContainer :config="config" class="aspect-auto h-44 w-full">
      <VisXYContainer :data="rows" :svg-defs="svgDefs" :y-domain="[0, top]" :margin="{ top: 8, right: 8 }">
        <!-- La seconde série (pic, envoyés) d'abord, pour que la première reste lisible par-dessus. -->
        <VisArea :x="x" :y="yB" :color="`url(#fill-b-${uid})`" :opacity="0.5" />
        <VisArea :x="x" :y="yA" :color="`url(#fill-a-${uid})`" :opacity="0.6" />
        <VisLine :x="x" :y="yB" :color="colors[1]" :line-width="1" :line-dash-array="[4, 3]" />
        <VisLine :x="x" :y="yA" :color="colors[0]" :line-width="1.5" />
        <!-- Avec un ou deux points, une ligne ne se voit pas : chaque point est marqué. -->
        <VisScatter v-if="rows.length <= 2" :x="x" :y="[yA, yB]" :size="7" :color="(_d: Row, i: number) => colors[i]" />
        <VisAxis type="x" :x="x" :tick-line="false" :domain-line="false" :grid-line="false" :num-ticks="4" :tick-format="tickX" />
        <VisAxis type="y" :num-ticks="3" :tick-line="false" :domain-line="false" :tick-format="(value: number) => format(value)" />
        <ChartTooltip />
        <ChartCrosshair :x="x" :y="[yA, yB]" :template="tooltip" :color="(_d: Row, i: number) => colors[i]" />
      </VisXYContainer>
    </ChartContainer>
  </div>
</template>
