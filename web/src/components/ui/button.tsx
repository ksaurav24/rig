import { cva, type VariantProps } from 'class-variance-authority'
import type { ComponentProps } from 'react'
import { Button as FluidButton } from '@/components/fluid/button'
import { cn } from '@/lib/utils'

/*
 * The app's Button keeps the shadcn call-site API (variant/size names,
 * `render`) but draws with Fluid Functionalism's button, so every button in
 * the dashboard shares one press/hover motion and one set of fills.
 *
 *   default → primary (ink)      outline → tertiary (hairline)
 *   secondary → secondary (tint) ghost → ghost   destructive → destructive
 */
type ShadVariant = 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive' | 'link'
type ShadSize = 'default' | 'xs' | 'sm' | 'lg' | 'icon' | 'icon-xs' | 'icon-sm' | 'icon-lg'

const VARIANT = {
  default: 'primary',
  outline: 'tertiary',
  secondary: 'secondary',
  ghost: 'ghost',
  destructive: 'destructive',
} as const

const SIZE = {
  default: 'compact',
  xs: 'compact',
  sm: 'compact',
  lg: 'default',
  icon: 'icon-compact',
  'icon-xs': 'icon-compact',
  'icon-sm': 'icon-compact',
  'icon-lg': 'icon',
} as const

// Inline icons passed as children sit in the label span; lay that span out
// as a row so `<Icon /> Label` keeps its gap and icons get a default size.
const LABEL_ROW =
  "[&_svg:not([class*='size-'])]:size-4 [&_svg]:shrink-0 [&>span>span]:inline-flex [&>span>span]:items-center [&>span>span]:gap-1.5"

type ButtonProps = Omit<ComponentProps<typeof FluidButton>, 'variant' | 'size'> & {
  variant?: ShadVariant
  size?: ShadSize
}

function Button({ variant = 'default', size = 'default', className, ...props }: ButtonProps) {
  if (variant === 'link') {
    return <FluidButton variant="ghost" size={SIZE[size]} className={cn(LABEL_ROW, 'h-auto px-0 text-foreground underline-offset-4 hover:underline [&>span:first-child]:hidden', className)} {...props} />
  }
  return <FluidButton data-slot="button" variant={VARIANT[variant]} size={SIZE[size]} className={cn(LABEL_ROW, className)} {...props} />
}

/** Class-only button look, for plain anchors and router links. */
const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg text-[13px] font-medium whitespace-nowrap transition-colors outline-none select-none focus-visible:ring-1 focus-visible:ring-[color:var(--focus-ring)] disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default: 'bg-foreground text-background hover:bg-foreground/90',
        outline: 'shadow-[0_0_0_1px_var(--border)] hover:bg-hover',
        secondary: 'bg-tint hover:bg-tint-hover',
        ghost: 'text-muted-foreground hover:bg-hover hover:text-foreground',
        destructive: 'bg-destructive/15 text-destructive hover:bg-destructive/25',
        link: 'underline-offset-4 hover:underline',
      },
      size: {
        default: 'h-7 px-3',
        xs: 'h-6 px-2 text-xs',
        sm: 'h-7 px-3',
        lg: 'h-9 px-4',
        icon: 'size-7',
        'icon-xs': 'size-6',
        'icon-sm': 'size-7',
        'icon-lg': 'size-9',
      },
    },
    defaultVariants: { variant: 'default', size: 'default' },
  },
)

export type { VariantProps }
export { Button, buttonVariants }
