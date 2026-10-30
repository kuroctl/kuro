import type { VariantProps } from "class-variance-authority"
import { cva } from "class-variance-authority"

export { default as Alert } from "./Alert.vue"
export { default as AlertDescription } from "./AlertDescription.vue"
export { default as AlertTitle } from "./AlertTitle.vue"

// Alerte du design system : bordure et fond teintés, icône à gauche, titre en couleur du texte.
export const alertVariants = cva(
  "flex items-start gap-3 rounded-lg border px-4 py-3.5 text-sm [&>svg]:mt-0.5 [&>svg]:size-4 [&>svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "border-border bg-card text-foreground [&>svg]:text-muted-foreground",
        info: "border-info/35 bg-info/8 [&>svg]:text-info",
        success: "border-success/35 bg-success/8 [&>svg]:text-success",
        warning: "border-warning/35 bg-warning/8 [&>svg]:text-warning",
        destructive: "border-destructive/35 bg-destructive/8 [&>svg]:text-destructive",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
)

export type AlertVariants = VariantProps<typeof alertVariants>
