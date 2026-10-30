// Alias des schémas générés depuis l'OpenAPI du backend (`pnpm gen:api`).
import type { components } from './schema'

export type Schemas = components['schemas']

export type ErrorCode = Schemas['ErrorCode']
export type Problem = Schemas['Problem']
export type FieldProblem = Schemas['FieldProblem']
