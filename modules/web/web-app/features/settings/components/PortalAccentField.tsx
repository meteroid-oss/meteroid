import { GenericFormField, Input, cn } from '@md/ui'
import { Check } from 'lucide-react'
import { ReactNode } from 'react'
import { Control, FieldPath, FieldValues } from 'react-hook-form'

import { DEFAULT_ACCENT, brandPresetsFor } from '@/pages/portal/experience/theme'

const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i

const normalize = (v: string | undefined): string => (v ?? '').trim().toLowerCase()

const Swatch = ({
  hex,
  title,
  selected,
  onClick,
}: {
  hex: string
  title: string
  selected: boolean
  onClick: () => void
}) => (
  <button
    type="button"
    title={title}
    aria-label={title}
    aria-pressed={selected}
    onClick={onClick}
    style={{ backgroundColor: hex }}
    className={cn(
      'relative h-6 w-6 rounded-full border border-border transition-transform hover:scale-110',
      'focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background',
      selected && 'ring-2 ring-ring ring-offset-2 ring-offset-background'
    )}
  >
    {selected && (
      <Check className="absolute inset-0 m-auto h-3 w-3 text-white drop-shadow-[0_1px_1px_rgba(0,0,0,0.7)]" />
    )}
  </button>
)

interface PortalAccentFieldProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> {
  name: TName
  control: Control<TFieldValues>
  /** Active portal theme mode — selects which per-mode swatch variant is shown. */
  themeMode?: string
  containerClassName?: string
  label?: string
  description?: string
  placeholder?: string
  /** Shown under the input, e.g. a contrast warning. */
  hint?: ReactNode
}

/**
 * Accent picker for the customer portal: a curated palette (shown for the active
 * theme mode) plus a custom hex input backed by a native color picker. Only the
 * variant matching the current mode is offered; a stored value that matches no
 * preset simply shows as custom. Blank inherits the default-entity accent.
 */
export const PortalAccentField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  name,
  control,
  themeMode,
  containerClassName,
  label = 'Accent color',
  description = 'Used for highlights and primary actions. Leave blank to inherit the default entity.',
  placeholder = DEFAULT_ACCENT,
  hint,
}: PortalAccentFieldProps<TFieldValues, TName>) => {
  const mode = themeMode === 'dark' ? 'dark' : 'light'
  const presets = brandPresetsFor(mode)

  return (
    <GenericFormField
      name={name}
      control={control}
      label={label}
      description={description}
      containerClassName={containerClassName}
      render={({ field }) => {
        const value = (field.value as string | undefined) ?? ''
        const current = normalize(value)
        const isValidHex = HEX.test(value)
        const colorInputValue = isValidHex ? value : DEFAULT_ACCENT

        return (
          <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center gap-2">
              {presets.map(preset => (
                <Swatch
                  key={preset.label}
                  hex={preset.hex}
                  title={preset.label}
                  selected={current === normalize(preset.hex)}
                  onClick={() => field.onChange(preset.hex.toUpperCase())}
                />
              ))}
            </div>
            <div className="flex items-center gap-2">
              <label
                className="relative h-7 w-7 shrink-0 cursor-pointer overflow-hidden rounded-md border border-border"
                title="Custom color"
                style={{
                  background:
                    'conic-gradient(from 90deg, #f87171, #fbbf24, #34d399, #38bdf8, #a855f7, #f87171)',
                }}
              >
                <input
                  type="color"
                  value={colorInputValue}
                  onChange={e => field.onChange(e.target.value.toUpperCase())}
                  className="absolute -inset-2 h-[200%] w-[200%] cursor-pointer opacity-0"
                />
              </label>
              <Input
                value={value}
                placeholder={placeholder}
                onChange={e => field.onChange(e.target.value)}
                onBlur={field.onBlur}
                className="w-28 font-mono uppercase"
              />
              <span className="text-xs text-muted-foreground">Custom</span>
            </div>
            {hint}
          </div>
        )
      }}
    />
  )
}
