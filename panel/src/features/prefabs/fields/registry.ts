// Type de champ du prefab → composant de saisie shadcn-vue. Ajouter un type de champ (couleur,
// carte, liste…) = un composant respectant `FieldControlProps` + une entrée ici.
import type { Component } from 'vue'

import type { FieldType } from '../manifest'
import FieldNumber from './FieldNumber.vue'
import FieldSelect from './FieldSelect.vue'
import FieldSlider from './FieldSlider.vue'
import FieldSwitch from './FieldSwitch.vue'
import FieldText from './FieldText.vue'
import FieldTextarea from './FieldTextarea.vue'

export const fieldControls: Record<FieldType, Component> = {
  string: FieldText,
  password: FieldText,
  secret: FieldText,
  text: FieldTextarea,
  int: FieldNumber,
  float: FieldNumber,
  slider: FieldSlider,
  bool: FieldSwitch,
  enum: FieldSelect,
}
