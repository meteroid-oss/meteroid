import { VariantProps, cva } from 'class-variance-authority'
import {
  Control,
  ControllerFieldState,
  ControllerProps,
  ControllerRenderProps,
  FieldPath,
  FieldValues,
  UseFormStateReturn,
} from 'react-hook-form'

import { cn } from '@ui/lib'

import { FormControl, FormField, FormItem, FormLabel, FormMessage } from './form'

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
      horizontal: 'col-start-5 col-span-8',
    },
  },
}) satisfies typeof formFieldVariants

export const genericFormFieldVariants = {
  formField: formFieldVariants,
  label: formFieldLabelVariants,
  input: inputVariants,
  message: formFieldMessageVariants,
}

export type GenericFormFieldVariantProps = VariantProps<typeof genericFormFieldVariants.formField>

export interface GenericFormFieldProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<ControllerProps<TFieldValues, TName>, 'render'> {
  label?: string
  key?: string
  containerClassName?: string
  labelClassName?: string
  className?: string
  layout?: 'vertical' | 'horizontal' | null
  control: Control<TFieldValues>
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

export const GenericFormField = <
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
}: GenericFormFieldProps<TFieldValues, TName>) => {
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
