import { HTMLAttributes } from 'react'
import { FieldPath, FieldValues } from 'react-hook-form'

import { GenericFormFieldProps } from './generic-form-field'

export const destructuredFormProps = <
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
  T,
  A extends HTMLAttributes<T>,
>({
  label,
  name,
  rules,
  defaultValue,
  control,
  disabled,
  required,
  shouldUnregister,
  key,
  layout = 'vertical',
  containerClassName,
  labelClassName,
  className,
  description,
  rightLabel,
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
    required,
    layout,
    containerClassName,
    labelClassName,
    className,
    inputProps,
    rightLabel,
    description,
  }
}
