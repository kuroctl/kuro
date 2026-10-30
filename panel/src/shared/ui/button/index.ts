import type { VariantProps } from "class-variance-authority"
import { cva } from "class-variance-authority"

export { default as Button } from "./Button.vue"

export const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-lg border border-transparent text-sm font-semibold transition-[background-color,border-color,color,box-shadow,transform] active:translate-y-px disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 shrink-0 [&_svg]:shrink-0 outline-none focus-visible:ring-ring/50 focus-visible:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground shadow-[0_1px_2px_color-mix(in_oklch,var(--primary)_30%,transparent)] hover:bg-[color-mix(in_oklch,var(--primary)_88%,white)]",
        destructive:
          "bg-destructive text-destructive-foreground shadow-[0_1px_2px_color-mix(in_oklch,var(--destructive)_30%,transparent)] hover:bg-[color-mix(in_oklch,var(--destructive)_88%,white)] focus-visible:ring-destructive/30",
        success:
          "bg-success text-[oklch(0.2_0.04_155)] hover:bg-[color-mix(in_oklch,var(--success)_88%,white)]",
        outline:
          "border-input bg-transparent hover:bg-muted",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-[color-mix(in_oklch,var(--secondary)_85%,var(--foreground))]",
        ghost:
          "text-muted-foreground hover:bg-muted hover:text-foreground",
        link: "h-auto! border-0 px-0! text-[13px] text-primary underline-offset-3 hover:underline",
      },
      size: {
        "default": "h-10 px-4",
        "xs": "h-6 gap-1 rounded-md px-2 text-xs [&_svg:not([class*='size-'])]:size-3",
        "sm": "h-8 rounded-md gap-1.5 px-3 text-[13px]",
        "lg": "h-12 px-[22px] text-[15px]",
        "icon": "size-10",
        "icon-xs": "size-6 rounded-md [&_svg:not([class*='size-'])]:size-3",
        "icon-sm": "size-8 rounded-md",
        "icon-lg": "size-12",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
)
export type ButtonVariants = VariantProps<typeof buttonVariants>
