// Palette des badges de rôle, accordée au thème sombre ; toute autre couleur `#rrggbb` reste
// possible.
export const ROLE_COLORS = [
  '#64748b',
  '#ef4444',
  '#f97316',
  '#f59e0b',
  '#84cc16',
  '#22c55e',
  '#14b8a6',
  '#06b6d4',
  '#3b82f6',
  '#6366f1',
  '#a855f7',
  '#ec4899',
] as const

export const isHexColor = (value: string) => /^#[0-9a-fA-F]{6}$/.test(value)
