<script setup lang="ts">
// Badge d'un rôle, teinté de sa couleur : fond et bordure translucides, texte lisible en sombre
// comme en clair.
defineProps<{ name: string; color?: string }>()
</script>

<template>
  <span
    class="inline-flex w-fit shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full border px-2 py-0.5 text-xs font-medium"
    :style="{ borderColor: `${color ?? '#64748b'}66`, backgroundColor: `${color ?? '#64748b'}26` }"
  >
    <span class="size-2 rounded-full" :style="{ backgroundColor: color ?? '#64748b' }" aria-hidden="true" />
    {{ name }}
  </span>
</template>
