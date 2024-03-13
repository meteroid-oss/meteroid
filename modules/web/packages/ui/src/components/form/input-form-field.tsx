import { FieldValues, FieldPath, UseControllerProps, Control, PathValue } from 'react-hook-form'

import { Input } from '..'

import { GenericFormField, GenericFormFieldVariantProps } from './generic-form-field'
import { destructuredFormProps } from './utils'

interface Transformer<T> {
  fromInput: (value: string) => T
  toInput: (value: T | undefined) => string
}

interface InputFieldProps<TFieldValues extends FieldValues, TName extends FieldPath<TFieldValues>>
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'defaultValue' | 'name'>,
    UseControllerProps<TFieldValues, TName> {
  label?: string
  key?: string
  containerClassName?: string
  labelClassName?: string
  control: Control<TFieldValues>
  transformer?: Transformer<PathValue<TFieldValues, TName>>
}

const parseNumber = (value?: string) => (Number.isNaN(Number(value)) ? undefined : Number(value))

export const InputFormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  ...props
}: InputFieldProps<TFieldValues, TName> & GenericFormFieldVariantProps) => {
  const { inputProps, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      {...formFieldProps}
      render={({ field, className }) => {
        const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
          if (props.transformer) {
            const newValue = props.transformer.fromInput(e.target.value)
            field.onChange({ ...e, target: { ...e.target, value: newValue } })
            // valueAsNumber is not available with controller
          } else if (props.type == 'number') {
            const newValue = parseNumber(e.target.value)
            field.onChange({ ...e, target: { ...e.target, value: newValue } })
          } else {
            field.onChange(e)
          }
        }

        const value = props.transformer ? props.transformer.toInput(field.value) : field.value

        return (
          <Input
            {...field}
            {...inputProps}
            onChange={onChange}
            value={value}
            className={className}
          />
        )
      }}
    />
  )
}
