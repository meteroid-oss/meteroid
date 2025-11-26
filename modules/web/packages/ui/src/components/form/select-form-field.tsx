import { SelectProps } from '@radix-ui/react-select'
import { Control, FieldPath, FieldValues, UseControllerProps } from 'react-hook-form'

import { cn } from '@ui/lib'

import * as Select from '../ui/select'

import { GenericFormField, GenericFormFieldVariantProps } from './generic-form-field'
import { destructuredFormProps } from './utils'

type SelectFormFieldProps<T extends FieldValues, TName extends FieldPath<T>> = Omit<
  SelectProps,
  'defaultValue' | 'name'
> &
  UseControllerProps<T, TName> & {
    label?: string
    key?: string
    className?: string
    containerClassName?: string
    labelClassName?: string
    contentClassName?: string
    placeholder?: string
    empty?: boolean
    labelTooltip?: React.ReactNode
    control: Control<T>
  }

export const SelectFormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  children,
  empty,
  ...props
}: SelectFormFieldProps<TFieldValues, TName> & GenericFormFieldVariantProps) => {
  const { inputProps, key, className, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      key={key}
      {...formFieldProps}
      className={cn('flex-row', className)}
      render={({ field, className }) => {
        return (
          <Select.Select
            {...inputProps}
            name={field.name}
            onValueChange={field.onChange}
            value={field.value}
          >
            <Select.SelectTrigger
              ref={field.ref}
              className={className}
              disabled={formFieldProps.disabled}
            >
              <Select.SelectValue placeholder={props.placeholder} />
            </Select.SelectTrigger>
            <Select.SelectContent className={props.contentClassName} hideWhenDetached>
              {empty && <Select.SelectEmpty />}
              {children}
            </Select.SelectContent>
          </Select.Select>
        )
      }}
    />
  )
}
