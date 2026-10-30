import type { VariantProps } from "class-variance-authority"
import { cva } from "class-variance-authority"

export { default as Badge } from "./Badge.vue"

export const badgeVariants = cva(
  "inline-flex h-6 items-center justify-center gap-1.5 rounded-full px-2.5 text-xs font-semibold w-fit whitespace-nowrap shrink-0 [&>svg]:size-3.5 [&>svg]:pointer-events-none focus-visible:ring-ring/50 focus-visible:ring-3 transition-[color,box-shadow] overflow-hidden",
  {
    variants: {
      variant: {
        default:
          "bg-primary/18 text-accent-foreground [a&]:hover:bg-primary/25",
        secondary:
          "bg-secondary text-secondary-foreground [a&]:hover:bg-secondary/90",
        destructive:
          "bg-destructive/16 text-destructive [a&]:hover:bg-destructive/25",
        success:
          "bg-success/16 text-success",
        warning:
          "bg-warning/18 text-warning",
        info:
          "bg-info/16 text-info",
        outline:
          "border border-border text-foreground [a&]:hover:bg-muted",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
)
export type BadgeVariants = VariantProps<typeof badgeVariants>
