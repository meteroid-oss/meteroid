import { Control, FieldPath, FieldValues, PathValue, UseControllerProps } from 'react-hook-form'

import { Input, InputProps } from '..'

import { GenericFormField, GenericFormFieldVariantProps } from './generic-form-field'
import { destructuredFormProps } from './utils'

interface Transformer<T> {
  fromInput: (value: string) => T
  toInput: (value: T | undefined) => string
}

interface InputFieldProps<TFieldValues extends FieldValues, TName extends FieldPath<TFieldValues>>
  extends Omit<InputProps, 'defaultValue' | 'name'>,
    UseControllerProps<TFieldValues, TName> {
  label?: string
  description?: string
  key?: string
  containerClassName?: string
  labelClassName?: string
  control: Control<TFieldValues>
  transformer?: Transformer<PathValue<TFieldValues, TName>>
  asString?: boolean
  rightLabel?: React.ReactNode
}

const parseNumber = (value?: string) => (Number.isNaN(Number(value)) ? undefined : Number(value))

export const InputFormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  ...props
}: InputFieldProps<TFieldValues, TName> & GenericFormFieldVariantProps) => {
  const { inputProps, key, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      key={key}
      {...formFieldProps}
      render={({ field, className }) => {
        const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
          if (props.transformer) {
            const newValue = props.transformer.fromInput(e.target.value)
            field.onChange({ ...e, target: { ...e.target, value: newValue } })
            // valueAsNumber is not available with controller
          } else if (props.type == 'number' && !props.asString) {
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
            wrapperClassName={
              inputProps.rightText && props.layout == 'horizontal' ? 'col-span-8' : undefined
            }
          />
        )
      }}
    />
  )
}
