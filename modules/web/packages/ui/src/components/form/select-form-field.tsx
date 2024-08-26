import { SelectProps } from '@radix-ui/react-select'
import { Control, FieldPath, FieldValues, UseControllerProps } from 'react-hook-form'

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
  const { inputProps, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      {...formFieldProps}
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
