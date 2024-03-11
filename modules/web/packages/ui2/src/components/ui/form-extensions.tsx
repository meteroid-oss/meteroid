import {
  FieldValues,
  FieldPath,
  RegisterOptions,
  FieldPathValue,
  Control,
  UseControllerProps,
  ControllerFieldState,
  ControllerRenderProps,
  UseFormStateReturn,
  ControllerProps,
} from 'react-hook-form'
import { FormItem, FormField, FormLabel, FormControl, FormMessage } from './form'
import { Input as BaseInput } from './input'
import { Textarea as BaseTextarea } from './textarea'
import { Checkbox as BaseCheckbox } from './checkbox'
import * as BaseSelect from './select'
import * as SelectPrimitive from '@radix-ui/react-select'

import { VariantProps, cva } from 'class-variance-authority'
import { cn } from '../../lib'
import { HTMLAttributes, useMemo } from 'react'
import React from 'react'

const formFieldVariants = cva('', {
  variants: {
    layout: {
      vertical: '',
      horizontal: 'space-y-0 grid gap-2 md:grid md:grid-cols-12',
    },
  },
})
const formFieldLabelVariants = cva('', {
  variants: {
    layout: {
      vertical: '',
      horizontal: 'col-span-4 text-muted-foreground ',
    },
  },
}) satisfies typeof formFieldVariants

const inputVariants = cva('', {
  variants: {
    layout: {
      vertical: '',
      horizontal: 'col-span-8 flex flex-col gap-2',
    },
  },
}) satisfies typeof formFieldVariants

const formFieldMessageVariants = cva('', {
  variants: {
    layout: {
      vertical: '',
      horizontal: 'col-span-12',
    },
  },
}) satisfies typeof formFieldVariants

const parseNumber = (value?: string) => (Number.isNaN(Number(value)) ? undefined : Number(value))

interface SimpleFormFieldProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<ControllerProps<TFieldValues, TName>, 'render'> {
  label?: string
  key?: string
  containerClassName?: string
  labelClassName?: string
  className?: string
  layout?: 'vertical' | 'horizontal' | null
  render: ({
    field,
    fieldState,
    formState,
    className,
  }: {
    field: ControllerRenderProps<TFieldValues, TName>
    fieldState: ControllerFieldState
    formState: UseFormStateReturn<TFieldValues>
    className: string
  }) => React.ReactElement
}

const GenericFormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  render,
  label,
  layout = 'vertical',
  containerClassName,
  labelClassName,
  className,
  ...props
}: SimpleFormFieldProps<TFieldValues, TName>) => {
  return (
    <FormField
      {...props}
      render={fieldProps => (
        <FormItem className={cn(formFieldVariants({ layout }), containerClassName)}>
          {label && (
            <FormLabel className={cn(formFieldLabelVariants({ layout }), labelClassName)}>
              {label}
            </FormLabel>
          )}
          <FormControl>
            {render({ ...fieldProps, className: cn(inputVariants({ layout }), className) })}
          </FormControl>
          <FormMessage className={formFieldMessageVariants({ layout })} />
        </FormItem>
      )}
    />
  )
}

interface InputFieldProps<TFieldValues extends FieldValues, TName extends FieldPath<TFieldValues>>
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'defaultValue' | 'name'>,
    UseControllerProps<TFieldValues, TName> {
  label: string
  key?: string
  containerClassName?: string
  labelClassName?: string
}

const destructuredFormProps = <
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
}: Omit<SimpleFormFieldProps<TFieldValues, TName>, 'render'> & A) => {
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

const Input = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  ...props
}: InputFieldProps<TFieldValues, TName> & VariantProps<typeof formFieldVariants>) => {
  const { inputProps, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      {...formFieldProps}
      render={({ field, className }) => {
        const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
          // valueAsNumber is not available with controller
          if (props.type == 'number') {
            const newValue = parseNumber(e.target.value)
            field.onChange({ ...e, target: { ...e.target, value: newValue } })
          } else {
            field.onChange(e)
          }
        }

        return <BaseInput {...field} {...inputProps} onChange={onChange} className={className} />
      }}
    />
  )
}

interface TextareaFieldProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<React.TextareaHTMLAttributes<HTMLTextAreaElement>, 'defaultValue' | 'name'>,
    UseControllerProps<TFieldValues, TName> {
  label: string
  key?: string
  containerClassName?: string
  labelClassName?: string
}

const Textarea = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  ...props
}: TextareaFieldProps<TFieldValues, TName> & VariantProps<typeof formFieldVariants>) => {
  const { inputProps, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      {...formFieldProps}
      render={({ field, className }) => {
        return <BaseTextarea {...field} {...inputProps} className={className} />
      }}
    />
  )
}

type SelectProps<T extends FieldValues, TName extends FieldPath<T>> = Omit<
  SelectPrimitive.SelectProps,
  'defaultValue' | 'name'
> &
  UseControllerProps<T, TName> & {
    label: string
    key?: string
    className?: string
    containerClassName?: string
    labelClassName?: string
    contentClassName?: string
    placeholder?: string
    empty?: boolean
  }

const Select = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  children,
  empty,
  ...props
}: SelectProps<TFieldValues, TName> & VariantProps<typeof formFieldVariants>) => {
  const { inputProps, ...formFieldProps } = destructuredFormProps(props)

  return (
    <GenericFormField
      {...formFieldProps}
      render={({ field, className }) => {
        return (
          <BaseSelect.Select
            {...inputProps}
            name={field.name}
            onValueChange={field.onChange}
            value={field.value}
          >
            <BaseSelect.SelectTrigger ref={field.ref} className={className}>
              <BaseSelect.SelectValue placeholder={props.placeholder} />
            </BaseSelect.SelectTrigger>
            <BaseSelect.SelectContent className={props.contentClassName}>
              {empty && <BaseSelect.SelectEmpty />}
              {children}
            </BaseSelect.SelectContent>
          </BaseSelect.Select>
        )
      }}
    />
  )
}

interface CheckboxFieldProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<React.ComponentPropsWithoutRef<typeof BaseCheckbox>, 'defaultValue' | 'name'>,
    UseControllerProps<TFieldValues, TName> {
  label: string
  key?: string
  containerClassName?: string
  labelClassName?: string
}

const checkboxVariants = cva('flex flex-row items-start space-x-3 space-y-0', {
  variants: {
    variant: {
      default: 'py-4',
      card: 'rounded-md border p-4 shadow',
    },
  },
})

const Checkbox = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  label,
  name,
  rules,
  defaultValue,
  control,
  disabled,
  shouldUnregister,
  key,
  containerClassName,
  labelClassName,
  className,
  variant = 'default',
  ...props
}: CheckboxFieldProps<TFieldValues, TName> & VariantProps<typeof checkboxVariants>) => {
  return (
    <FormField
      control={control}
      name={name}
      rules={rules}
      defaultValue={defaultValue}
      disabled={disabled}
      key={key}
      shouldUnregister={shouldUnregister}
      render={({ field }) => (
        <FormItem className={cn(checkboxVariants({ variant }), containerClassName)}>
          <FormControl>
            <BaseCheckbox
              {...props}
              checked={field.value}
              onCheckedChange={field.onChange}
              className={cn('', className)}
            />
          </FormControl>

          <div className="space-y-1 leading-none">
            <FormLabel className={cn('', labelClassName)}>{label}</FormLabel>
            <FormMessage />
          </div>
        </FormItem>
      )}
    />
  )
}

export {
  Input as FormInput,
  Checkbox as FormCheckbox,
  Textarea as FormTextarea,
  GenericFormField,
  Select as FormSelect,
}
