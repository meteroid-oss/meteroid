import { InputProps, cn, useFormField } from '@md/ui'
import { forwardRef, useMemo } from 'react'

type BaseProps = {
  currency: string
  onBlur?: (e: React.FocusEvent) => void
  onFocus?: (e: React.FocusEvent) => void
  disabled?: boolean
  placeholder?: string
  showCurrency?: boolean
  className?: string
  inputClassName?: string
  precision?: number
  /** Shown after the amount instead of `currency` (e.g. "credits" for a credit unit). */
  unitLabel?: string
}

export const POSITIVE_NUM_REGEX = /^\d*\.?\d*$/

type UncontrolledProps = Omit<InputProps, 'name' | 'defaultValue'> & Omit<BaseProps, 'control'>

export const UncontrolledPriceInput = forwardRef<HTMLInputElement, UncontrolledProps>(
  (
    {
      currency,
      showCurrency = true,
      className,
      placeholder = '0.00',
      value,
      onChange,
      precision = 2,
      disabled,
      inputClassName,
      unitLabel,
      ...props
    },
    ref
  ) => {
    const { error } = useFormField()

    const handleBlur = (e: React.FocusEvent<HTMLInputElement>) => {
      const numValue = e.target.valueAsNumber
      if (!isNaN(numValue)) {
        e.target.value = new Intl.NumberFormat('en-US', {
          // A credit unit can have fewer than two decimals.
          minimumFractionDigits: Math.min(2, precision),
          maximumFractionDigits: precision,
          useGrouping: false,
        }).format(numValue)
        onChange?.(e)
      }
      props.onBlur?.(e)
    }

    const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
      // Pass through the change event - let browser handle localization
      onChange?.(e)
    }

    const displaySymbol = useMemo(() => {
      // A credit unit code (4+ characters) is no ISO currency: it has no symbol.
      try {
        const formatter = new Intl.NumberFormat('en-US', {
          style: 'currency',
          currency: currency,
          minimumFractionDigits: 2,
        })
        return formatter.format(0).replace(/\d|\./g, '').trim();
      } catch {
        return ''
      }
    }, [currency])

    return (
      <div
        className={cn(
          'relative rounded-md  ',
          error ? 'border-destructive border focus:ring-destructive' : '',
          className
        )}
      >
        {displaySymbol && (
          <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
            <span className="text-muted-foreground sm:text-sm">{displaySymbol}</span>
          </div>
        )}
        <input
          {...props}
          ref={ref}
          value={value}
          type="number"
          min="0"
          className={cn(
            displaySymbol ? 'pl-8' : '',
            'py-1.5 pl-8 pr-2 bg-input block w-full sm:text-sm border border-border rounded-md focus:outline-none focus:ring-1 focus:ring-ring placeholder:text-muted-foreground',
            disabled ? 'opacity-50' : '',
            showCurrency ? 'pr-12' : 'text-right',
            inputClassName
          )}
          onChange={handleChange}
          onBlur={handleBlur}
          placeholder={placeholder}
          aria-describedby="price-currency"
          disabled={disabled}
        />
        {showCurrency && (
          <div className="absolute inset-y-0 right-0 pr-3 flex items-center pointer-events-none">
            <span className="text-muted-foreground sm:text-sm" id="price-currency">
              {unitLabel ?? currency}
            </span>
          </div>
        )}
      </div>
    )
  }
)
