import { type ClassValue, clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'

/** Fusion de classes Tailwind (convention shadcn-vue). */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
