import { GenericFormFieldProps } from './generic-form-field'
import { HTMLAttributes } from 'react'
import { FieldPath, FieldValues } from 'react-hook-form'

export const destructuredFormProps = <
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
  A extends HTMLAttributes<any>,
>({
  label,
  name,
  rules,
  defaultValue,
  control,
  disabled,
  shouldUnregister,
  key,
  layout = 'vertical',
  containerClassName,
  labelClassName,
  className,
  ...inputProps
}: Omit<GenericFormFieldProps<TFieldValues, TName>, 'render'> & A) => {
  return {
    label,
    name,
    rules,
    defaultValue,
    control,
    disabled,
    shouldUnregister,
    key,
    layout,
    containerClassName,
    labelClassName,
    className,
    inputProps,
  }
}
