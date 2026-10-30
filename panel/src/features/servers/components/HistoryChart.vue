<script setup lang="ts">
// Courbe d'historique (SVG) : une ou deux séries horodatées, de 0 à `max` (ou au plus haut
// point), avec quatre repères de temps et la valeur du dernier point en légende.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps<{
  /** Points dans l'ordre du temps. */
  points: { at: string; values: number[] }[]
  /** Libellé de chaque série. */
  labels: string[]
  /** Formatage d'une valeur pour la légende. */
  format: (value: number) => string
  max?: number
  /** Plage affichée, pour le format des repères de temps. */
  range: '24h' | '7d' | '90d'
}>()

const WIDTH = 600
const HEIGHT = 160
const PAD = 4
const tones = ['stroke-primary', 'stroke-muted-foreground']

const { locale } = useI18n()

const top = computed(() => props.max ?? Math.max(1, ...props.points.flatMap((p) => p.values)))

const paths = computed(() => {
  const count = props.points.length
  if (count === 0) return []
  const step = (WIDTH - 2 * PAD) / Math.max(1, count - 1)
  return props.labels.map((_, series) =>
    props.points
      .map((point, index) => {
        const value = Math.min(point.values[series] ?? 0, top.value)
        const x = PAD + index * step
        const y = HEIGHT - PAD - (value / top.value) * (HEIGHT - 2 * PAD)
        return `${x.toFixed(1)},${y.toFixed(1)}`
      })
      .join(' '),
  )
})

const ticks = computed(() => {
  const count = props.points.length
  if (count < 2) return []
  const formatter = new Intl.DateTimeFormat(locale.value, props.range === '24h' ? { hour: '2-digit', minute: '2-digit' } : { day: 'numeric', month: 'short' })
  return [0, 1, 2, 3].map((i) => {
    const index = Math.round((i * (count - 1)) / 3)
    return { x: (index / Math.max(1, count - 1)) * 100, label: formatter.format(new Date(props.points[index]!.at)) }
  })
})

const latest = computed(() => props.points.at(-1)?.values ?? [])
</script>

<template>
  <div class="grid gap-1">
    <div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
      <span v-for="(label, index) in labels" :key="label" class="flex items-center gap-1.5">
        <span class="inline-block h-0.5 w-3 rounded" :class="index === 0 ? 'bg-primary' : 'bg-muted-foreground'" aria-hidden="true" />
        {{ label }}
        <span v-if="latest[index] !== undefined" class="font-semibold tabular-nums text-foreground">{{ format(latest[index]!) }}</span>
      </span>
    </div>
    <svg :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" preserveAspectRatio="none" class="h-40 w-full rounded-md bg-muted/40" aria-hidden="true">
      <polyline v-for="(d, index) in paths" :key="index" :points="d" fill="none" stroke-width="1.5" vector-effect="non-scaling-stroke" :class="tones[index] ?? tones[0]" />
    </svg>
    <div class="relative h-4 text-[11px] text-muted-foreground">
      <span v-for="tick in ticks" :key="tick.x" class="absolute -translate-x-1/2 whitespace-nowrap" :style="{ left: `${tick.x}%` }">{{ tick.label }}</span>
    </div>
  </div>
</template>
