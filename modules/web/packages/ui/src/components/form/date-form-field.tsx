import { ComponentProps } from 'react'
import { Control, FieldPath, FieldValues, UseControllerProps } from 'react-hook-form'

import { DatePicker } from '..'

import { GenericFormField, GenericFormFieldVariantProps } from './generic-form-field'
import { destructuredFormProps } from './utils'

interface DateFormFieldProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<
      ComponentProps<typeof DatePicker>,
      'defaultValue' | 'name' | 'disabled' | 'hidden' | 'mode' | 'onSelect'
    >,
    UseControllerProps<TFieldValues, TName> {
  label: string
  key?: string
  containerClassName?: string
  labelClassName?: string
  control: Control<TFieldValues>
  description?: string
}

export const DateFormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  ...props
}: DateFormFieldProps<TFieldValues, TName> & GenericFormFieldVariantProps) => {
  const { inputProps, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      {...formFieldProps}
      render={({ field, className }) => {
        return (
          <DatePicker
            captionLayout="dropdown"
            {...field}
            {...inputProps}
            mode="single"
            selected={field.value}
            date={field.value}
            onSelect={field.onChange}
            className={className}
          />
        )
      }}
    />
  )
}
