<script setup lang="ts">
// Choix de la langue hors coque (connexion, mise en route) : la préférence est mémorisée comme
// depuis le menu du compte. Chaque langue est nommée dans sa propre langue, pour être trouvée
// par qui ne lit pas celle affichée.
import { Languages } from '@lucide/vue'
import { useI18n } from 'vue-i18n'

import { applyLocale, type Locale, SUPPORTED_LOCALES } from '@/shared/i18n'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

const { t, locale } = useI18n()

/** Nom d'une langue, dans cette langue. */
const name = (code: Locale) => t(`preferences.languages.${code}`, {}, { locale: code })
</script>

<template>
  <!-- Un élément racine : la classe de position passée par la page s'y pose. -->
  <div>
    <Select :model-value="locale" @update:model-value="(value) => applyLocale(value as Locale)">
      <SelectTrigger class="h-8 gap-1.5 text-xs" :aria-label="t('preferences.language')">
        <Languages class="size-3.5 text-muted-foreground" aria-hidden="true" />
        <SelectValue>{{ name(locale as Locale) }}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem v-for="code in SUPPORTED_LOCALES" :key="code" :value="code" :lang="code">{{ name(code) }}</SelectItem>
      </SelectContent>
    </Select>
  </div>
</template>
