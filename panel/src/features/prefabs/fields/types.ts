import type { Field } from '../manifest'

/** Interface commune des composants de saisie : un nouveau type de champ = un composant + une
 * entrée du registre. */
export interface FieldControlProps {
  /** Fourni par `FormControl` : relie le libellé à la saisie. */
  id?: string
  field: Field
  modelValue: unknown
  disabled?: boolean
  /** Libellé traduit d'une option (`enum`). */
  optionLabel: (value: unknown) => string
}
