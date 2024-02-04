import { InputProps } from '@ui/components/Input'
import { cn } from '@ui/lib'
import { useEffect, useMemo, useState } from 'react'
import { Control, FieldValues, UseControllerProps, useController } from 'react-hook-form'

type BaseProps<T extends FieldValues> = {
  currency: string
  onBlur?: (e: React.FocusEvent) => void
  onFocus?: (e: React.FocusEvent) => void
  disabled?: boolean
  placeholder?: string
  showCurrency?: boolean
  className?: string
  control: Control<T>
  precision?: number
}

export const POSITIVE_NUM_REGEX = /^\d*\.?\d*$/

type Props<T extends FieldValues> = Omit<InputProps, 'name' | 'onValueChange' | 'ref' | 'value'> &
  UseControllerProps<T> &
  BaseProps<T>

const PriceInput = <T extends FieldValues>({
  currency,
  showCurrency = true,
  className,
  placeholder,
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  size: _size,
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  defaultValue: _defaultValue,
  precision = 2,
  error,
  ...props
}: Props<T>) => {
  const { field } = useController({ ...props })

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
        minimumFractionDigits: 2,
        maximumFractionDigits: precision,
      })
      .replaceAll(',', '')
      .replaceAll(' ', '')}` // TODO currencyjs or something
  }

  // const precisionFormatter = useMemo(() => {
  //   const formatter = new Intl.NumberFormat('en-US', {
  //     minimumFractionDigits: 2,
  //     maximumFractionDigits: precision,
  //   })
  //   return formatter
  // }, [precision])

  const handleBlur = () => {
    const formattedValue =
      inputValue && POSITIVE_NUM_REGEX.test(inputValue)
        ? formatCurrencyAmountWithoutRounding(parseFloat(inputValue)) //precisionFormatter.format(parseFloat(inputValue))
        : inputValue
    setInputValue(formattedValue)
    field.onChange(formattedValue)
    // field.onBlur()
  }

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const { value } = e.target
    if (value && value.length && POSITIVE_NUM_REGEX.test(value)) {
      setInputValue(value)
    } else if (!value.length) {
      setInputValue('0.00')
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
        error ? 'border-red-900 border focus:ring-red-500' : '',
        className
      )}
    >
      {displaySymbol && (
        <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
          <span className="text-slate-900 sm:text-sm">{displaySymbol}</span>
        </div>
      )}
      <input
        {...props}
        ref={field.ref}
        value={inputValue}
        type="text"
        className={cn(
          displaySymbol ? 'pl-8' : '',
          'py-1.5 pl-8 pr-2 block w-full sm:text-sm border border-slate-600 rounded-md focus:outline-none focus:ring-1 focus:ring-slate-600 focus:border-slate-600 ',
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
          <span className="text-slate-900 sm:text-sm" id="price-currency">
            {currency}
          </span>
        </div>
      )}
    </div>
  )
}

export default PriceInput
