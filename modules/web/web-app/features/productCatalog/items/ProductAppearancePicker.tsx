import { Popover, PopoverContent, PopoverTrigger, cn } from '@md/ui'
import { useState, type ReactNode } from 'react'

import {
  PRODUCT_COLORS,
  PRODUCT_ICONS,
  ProductIcon,
} from '@/features/productCatalog/items/ProductIcon'

import type { ProductAppearance } from '@/features/productCatalog/items/ProductIcon'

/**
 * The icons and colors to pick from, opened by the product's icon as a button, or by
 * `children` (a button) when given.
 */
export const ProductAppearancePicker = ({
  icon,
  color,
  onChange,
  disabled,
  size = 'md',
  children,
}: ProductAppearance & {
  onChange: (next: ProductAppearance) => void
  disabled?: boolean
  size?: 'sm' | 'md'
  children?: ReactNode
}) => {
  const [open, setOpen] = useState(false)
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        {children ?? (
          <button
            type="button"
            disabled={disabled}
            aria-label="Change the product's icon and color"
            className="rounded-md transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default"
          >
            <ProductIcon icon={icon} color={color} size={size} />
          </button>
        )}
      </PopoverTrigger>
      <PopoverContent align="start" className="w-auto space-y-3 p-3">
        <div className="grid grid-cols-8 gap-1" role="listbox" aria-label="Icon">
          {Object.entries(PRODUCT_ICONS).map(([name, Icon]) => (
            <button
              key={name}
              type="button"
              role="option"
              aria-selected={(icon ?? 'box') === name}
              aria-label={name}
              onClick={() => onChange({ icon: name, color })}
              className={cn(
                'flex size-7 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground',
                (icon ?? 'box') === name && 'bg-accent text-foreground ring-1 ring-border'
              )}
            >
              <Icon size={14} />
            </button>
          ))}
        </div>
        <div
          className="flex items-center gap-1.5 border-t border-border pt-3"
          role="listbox"
          aria-label="Color"
        >
          <Swatch
            label="Neutral"
            selected={!color}
            className="border border-border bg-background"
            onPick={() => onChange({ icon, color: undefined })}
          />
          {Object.entries(PRODUCT_COLORS).map(([name, { dot }]) => (
            <Swatch
              key={name}
              label={name}
              selected={color === name}
              className={dot}
              onPick={() => onChange({ icon, color: name })}
            />
          ))}
        </div>
      </PopoverContent>
    </Popover>
  )
}

const Swatch = ({
  label,
  selected,
  className,
  onPick,
}: {
  label: string
  selected: boolean
  className: string
  onPick: () => void
}) => (
  <button
    type="button"
    role="option"
    aria-selected={selected}
    aria-label={label}
    onClick={onPick}
    className={cn(
      'size-5 rounded-full ring-offset-2 ring-offset-popover',
      className,
      selected && 'ring-2 ring-ring'
    )}
  />
)
