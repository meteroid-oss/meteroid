import { VariantProps, cva } from 'class-variance-authority'
import { Control, FieldPath, FieldValues, UseControllerProps } from 'react-hook-form'

import { cn } from '@ui/lib'

import { Checkbox } from '..'

import { FormControl, FormDescription, FormField, FormItem, FormLabel, FormMessage } from './form'

interface CheckboxFieldProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<React.ComponentPropsWithoutRef<typeof Checkbox>, 'defaultValue' | 'name'>,
    UseControllerProps<TFieldValues, TName> {
  label: string
  key?: string
  description?: string
  containerClassName?: string
  labelClassName?: string
  control: Control<TFieldValues>
}

const checkboxVariants = cva('flex flex-row items-start space-x-3 space-y-0', {
  variants: {
    variant: {
      default: 'py-4',
      card: 'rounded-md border p-4 shadow',
    },
  },
})

export const CheckboxFormField = <
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
  description,
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
            <Checkbox
              {...props}
              checked={field.value}
              onCheckedChange={field.onChange}
              className={cn('', className)}
            />
          </FormControl>

          <div className="space-y-1 leading-none">
            <FormLabel className={cn('', labelClassName)}>{label}</FormLabel>
            {description && <FormDescription>{description}</FormDescription>}
            <FormMessage />
          </div>
        </FormItem>
      )}
    />
  )
}
