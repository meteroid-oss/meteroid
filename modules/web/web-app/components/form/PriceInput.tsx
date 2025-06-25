import { InputProps, cn, useFormField } from '@md/ui'
import { forwardRef, useEffect, useMemo, useState } from 'react'
import { Control, FieldValues, UseControllerProps, useController } from 'react-hook-form'

type BaseProps = {
  currency: string
  onBlur?: (e: React.FocusEvent) => void
  onFocus?: (e: React.FocusEvent) => void
  disabled?: boolean
  placeholder?: string
  showCurrency?: boolean
  className?: string
  precision?: number
}

type BaseControllerProps<T extends FieldValues> = BaseProps & {
  control: Control<T>
}

export const POSITIVE_NUM_REGEX = /^\d*\.?\d*$/

type Props<T extends FieldValues> = Omit<InputProps, 'name' | 'onValueChange' | 'ref' | 'value'> &
  UseControllerProps<T> &
  BaseControllerProps<T>
/**
 *
 * @deprecated
 * prefer using `UncontrolledPriceInput` instead
 */
const PriceInput = <T extends FieldValues>({
  currency,
  showCurrency = true,
  className,
  placeholder,

  precision = 2,
  ...props
}: Props<T>) => {
  const { field, fieldState } = useController(props)

  const [inputValue, setInputValue] = useState('')

  useEffect(() => {
    if (field.value) {
      setInputValue(field.value)
    }
  }, [field.value])

  const formatCurrencyAmountWithoutRounding = (amount: number) => {
    const negativeSign = Number(amount) < 0 ? '-' : ''
    return `${negativeSign}${Math.abs(Number(amount))
      .toLocaleString(undefined, {
        minimumFractionDigits: 0,
        maximumFractionDigits: precision,
      })
      .replaceAll(',', '')
      .replaceAll(' ', '')}` // TODO currencyjs or something
  }

  const handleBlur = () => {
    const formattedValue =
      inputValue && POSITIVE_NUM_REGEX.test(inputValue)
        ? formatCurrencyAmountWithoutRounding(parseFloat(inputValue))
        : inputValue
    setInputValue(formattedValue)
    field.onChange(formattedValue)
  }

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const { value } = e.target
    if (value && value.length && POSITIVE_NUM_REGEX.test(value)) {
      setInputValue(value)
    } else if (!value.length) {
      setInputValue('')
    }
  }

  const displaySymbol = useMemo(() => {
    const formatter = new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: currency,
      minimumFractionDigits: 2,
    })
    return formatter.format(0).replace(/\d|\./g, '').trim()
  }, [currency])

  return (
    <div
      className={cn(
        'relative rounded-md  ',
        fieldState.error ? 'border-destructive border focus:ring-destructive' : '',
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
        ref={field.ref}
        value={inputValue}
        type="text"
        className={cn(
          displaySymbol ? 'pl-8' : '',
          'py-1.5 pl-8 pr-2 bg-input block w-full sm:text-sm border border-border rounded-md focus:outline-none focus:ring-1 focus:ring-ring ',
          field.disabled ? 'opacity-50' : '',
          showCurrency ? 'pr-12' : 'text-right'
        )}
        onBlur={handleBlur}
        onChange={handleChange}
        placeholder={placeholder}
        aria-describedby="price-currency"
        disabled={field.disabled}
      />
      {showCurrency && (
        <div className="absolute inset-y-0 right-0 pr-3 flex items-center pointer-events-none">
          <span className="text-muted-foreground sm:text-sm" id="price-currency">
            {currency}
          </span>
        </div>
      )}
    </div>
  )
}

type UncontrolledProps = Omit<InputProps, 'name' | 'defaultValue'> & Omit<BaseProps, 'control'>

export const UncontrolledPriceInput = forwardRef<HTMLInputElement, UncontrolledProps>(
  (
    {
      currency,
      showCurrency = true,
      className,
      placeholder,
      value,
      onChange,
      precision = 2,
      disabled,
      ...props
    },
    ref
  ) => {
    const { error } = useFormField()

    const [inputValue, setInputValue] = useState<string | undefined>(undefined)

    const formatCurrencyAmountWithoutRounding = (amount: number) => {
      const negativeSign = Number(amount) < 0 ? '-' : ''
      return `${negativeSign}${Math.abs(Number(amount))
        .toLocaleString(undefined, {
          minimumFractionDigits: 0,
          maximumFractionDigits: precision,
        })
        .replaceAll(',', '')
        .replaceAll(' ', '')}` // TODO currencyjs or something
    }

    const format = (value: string | undefined) =>
      value && POSITIVE_NUM_REGEX.test(value)
        ? formatCurrencyAmountWithoutRounding(parseFloat(value))
        : value
          ? value
          : ''

    useEffect(() => {
      if (inputValue === undefined) {
        setInputValue(format(value as string))
      } else if (value) {
        setInputValue(value as string)
      } else {
        setInputValue('')
      }
    }, [value])

    const handleBlur = () => {
      const formattedValue = format(inputValue)
      setInputValue(formattedValue)
      onChange?.({ target: { value: formattedValue } } as React.ChangeEvent<HTMLInputElement>)
    }

    const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
      const { value } = e.target
      if (value && POSITIVE_NUM_REGEX.test(value)) {
        setInputValue(value)
        onChange?.({ target: { value: value } } as React.ChangeEvent<HTMLInputElement>)
      } else if (!value.length) {
        setInputValue('')
        onChange?.({ target: { value: '' } } as React.ChangeEvent<HTMLInputElement>)
      }
    }

    const displaySymbol = useMemo(() => {
      const formatter = new Intl.NumberFormat('en-US', {
        style: 'currency',
        currency: currency,
        minimumFractionDigits: 2,
      })
      return formatter.format(0).replace(/\d|\./g, '').trim()
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
          value={inputValue}
          type="text"
          className={cn(
            displaySymbol ? 'pl-8' : '',
            'py-1.5 pl-8 pr-2 bg-input block w-full sm:text-sm border border-border rounded-md focus:outline-none focus:ring-1 focus:ring-ring ',
            disabled ? 'opacity-50' : '',
            showCurrency ? 'pr-12' : 'text-right'
          )}
          onBlur={handleBlur}
          onChange={handleChange}
          placeholder={placeholder}
          aria-describedby="price-currency"
          disabled={disabled}
        />
        {showCurrency && (
          <div className="absolute inset-y-0 right-0 pr-3 flex items-center pointer-events-none">
            <span className="text-muted-foreground sm:text-sm" id="price-currency">
              {currency}
            </span>
          </div>
        )}
      </div>
    )
  }
)

export default PriceInput
