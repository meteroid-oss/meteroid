import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Label,
  RadioGroup,
  RadioGroupItem,
  cn,
} from '@md/ui'
import { useId } from 'react'

import type { ReactNode } from 'react'
import type { Control, FieldPath, FieldValues } from 'react-hook-form'

export type ChoiceCardOption<T extends string = string> = {
  value: T
  label: string
  hint?: string
  icon?: ReactNode
  disabled?: boolean
}

type ChoiceCardsProps<T extends string> = {
  value: T | undefined
  onChange: (value: T) => void
  options: readonly ChoiceCardOption<T>[]
  columns?: 1 | 2 | 3 | 4
  disabled?: boolean
  className?: string
}

const COLUMNS = {
  1: 'grid-cols-1',
  2: 'sm:grid-cols-2',
  3: 'sm:grid-cols-3',
  4: 'sm:grid-cols-2 lg:grid-cols-4',
} as const

export const ChoiceCards = <T extends string>({
  value,
  onChange,
  options,
  columns = 2,
  disabled,
  className,
}: ChoiceCardsProps<T>) => {
  const id = useId()
  return (
    <RadioGroup
      value={value}
      onValueChange={v => onChange(v as T)}
      disabled={disabled}
      className={cn('grid gap-2', COLUMNS[columns], className)}
    >
      {options.map(option => {
        const itemId = `${id}-${option.value}`
        return (
          <Label
            key={option.value}
            htmlFor={itemId}
            className="flex cursor-pointer items-start gap-2 rounded-md border border-border p-3 font-normal has-[[data-state=checked]]:border-primary has-[:disabled]:cursor-not-allowed has-[:disabled]:opacity-60"
          >
            <RadioGroupItem
              value={option.value}
              id={itemId}
              className="mt-0.5"
              disabled={disabled || option.disabled}
            />
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
              <span className="flex items-center gap-1.5 text-sm font-medium">
                {option.icon}
                {option.label}
              </span>
              {option.hint && (
                <span className="text-xs text-muted-foreground">{option.hint}</span>
              )}
            </span>
          </Label>
        )
      })}
    </RadioGroup>
  )
}

type ChoiceCardsFormFieldProps<TFieldValues extends FieldValues, T extends string> = Omit<
  ChoiceCardsProps<T>,
  'value' | 'onChange'
> & {
  control: Control<TFieldValues>
  name: FieldPath<TFieldValues>
  label?: string
  onValueChange?: (value: T) => void
}

export const ChoiceCardsFormField = <TFieldValues extends FieldValues, T extends string>({
  control,
  name,
  label,
  onValueChange,
  ...props
}: ChoiceCardsFormFieldProps<TFieldValues, T>) => (
  <FormField
    control={control}
    name={name}
    render={({ field }) => (
      <FormItem className="space-y-2">
        {label && <FormLabel>{label}</FormLabel>}
        <FormControl>
          <ChoiceCards
            {...props}
            value={field.value}
            onChange={v => {
              field.onChange(v)
              onValueChange?.(v)
            }}
          />
        </FormControl>
        <FormMessage />
      </FormItem>
    )}
  />
)
