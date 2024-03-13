import { FieldValues, FieldPath, UseControllerProps, Control } from 'react-hook-form'

import { Textarea } from '..'

import { GenericFormField, GenericFormFieldVariantProps } from './generic-form-field'
import { destructuredFormProps } from './utils'

interface TextareaFieldProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<React.TextareaHTMLAttributes<HTMLTextAreaElement>, 'defaultValue' | 'name'>,
    UseControllerProps<TFieldValues, TName> {
  label: string
  key?: string
  containerClassName?: string
  labelClassName?: string
  control: Control<TFieldValues>
}

export const TextareaFormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  ...props
}: TextareaFieldProps<TFieldValues, TName> & GenericFormFieldVariantProps) => {
  const { inputProps, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      {...formFieldProps}
      render={({ field, className }) => {
        return <Textarea {...field} {...inputProps} className={className} />
      }}
    />
  )
}
