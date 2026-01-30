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
  layout?: 'vertical' | 'horizontal'
}

const checkboxVariants = cva('flex flex-row items-start space-x-3 space-y-0', {
  variants: {
    variant: {
      default: 'py-4',
      card: 'rounded-md border p-4 shadow',
    },
    layout: {
      vertical: '',
      horizontal: 'grid gap-2 md:grid-cols-12 items-center py-0',
    },
  },
})

const labelVariants = cva('', {
  variants: {
    layout: {
      vertical: '',
      horizontal: 'col-span-4 text-muted-foreground dark:text-muted-foreground font-normal text-xs',
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
  layout = 'vertical',
  ...props
}: CheckboxFieldProps<TFieldValues, TName> & VariantProps<typeof checkboxVariants>) => {
  const isHorizontal = layout === 'horizontal'

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
        <FormItem
          className={cn(checkboxVariants({ variant: isHorizontal ? undefined : variant, layout }), containerClassName)}
        >
          {isHorizontal ? (
            <>
              <FormLabel className={cn(labelVariants({ layout }), labelClassName)}>{label}</FormLabel>
              <div className="col-span-8 flex items-center gap-3">
                <FormControl>
                  <Checkbox
                    {...props}
                    checked={field.value}
                    onCheckedChange={field.onChange}
                    className={cn('', className)}
                  />
                </FormControl>
                {description && (
                  <FormDescription className="!mt-0 text-xs">{description}</FormDescription>
                )}
              </div>
              <FormMessage className="col-start-5 col-span-8" />
            </>
          ) : (
            <>
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
            </>
          )}
        </FormItem>
      )}
    />
  )
}
