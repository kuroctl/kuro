// Partie du manifest de prefab (format v1, ADR-0011) dont le panel a besoin. Le JSON Schema
// publié (`shared/prefab/schema/prefab.v1.schema.json`) reste la référence complète.

export type FieldType =
  | 'string'
  | 'text'
  | 'int'
  | 'float'
  | 'bool'
  | 'enum'
  | 'password'
  | 'secret'
  | 'slider'

export interface Field {
  type: FieldType
  default?: unknown
  required?: boolean
  min?: number | null
  max?: number | null
  step?: number | null
  pattern?: string | null
  max_length?: number | null
  options?: (string | number)[] | null
  visibility?: 'admin' | 'customer'
  editable?: 'always' | 'deploy'
  restart?: boolean
  generate?: { length: number } | null
}

export interface Group {
  icon?: string | null
  /** Ordre du fichier = ordre d'affichage. */
  fields: Record<string, Field>
}

export type SectionKey = 'general' | 'advanced'

/** Action d'un prefab (ADR-0039) : commande typée, paramètres comme des settings. */
export interface ActionDecl {
  service: string
  command: string[]
  params?: Record<string, Field>
  timeout?: number
  schedulable?: boolean
}

export interface Manifest {
  id: string
  version: string
  category: string
  /** Jeu sur le magasin Steam : le Studio en importe les images. */
  steam?: { app: number }
  services: Record<
    string,
    {
      image?: string
      ports?: Record<string, { container: number; protocol?: string; public?: boolean }>
      volumes?: Record<string, { path: string }>
    }
  >
  settings?: Partial<Record<SectionKey, Record<string, Group>>>
  files?: Record<string, { service: string; path: string; format: string; mode: string; editable?: boolean }>
  content?: Record<string, { kind: string; service: string; register?: { file: string }[] }>
  actions?: Record<string, ActionDecl>
}

/** Actions planifiables : déclarées `schedulable`, sans paramètre obligatoire sans défaut. */
export function schedulableActions(manifest: Manifest | undefined): string[] {
  return Object.entries(manifest?.actions ?? {})
    .filter(
      ([, action]) =>
        action.schedulable &&
        !Object.values(action.params ?? {}).some((field) => field.required && (field.default === undefined || field.default === null)),
    )
    .map(([key]) => key)
}

export const SECTIONS: SectionKey[] = ['general', 'advanced']

/** Ports publics du prefab : ceux qui reçoivent une allocation. */
export function publicPorts(manifest: Manifest): string[] {
  return Object.values(manifest.services).flatMap((service) =>
    Object.entries(service.ports ?? {})
      .filter(([, port]) => port.public)
      .map(([name]) => name),
  )
}
