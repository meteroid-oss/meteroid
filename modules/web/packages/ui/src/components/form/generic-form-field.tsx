import { VariantProps, cva } from 'class-variance-authority'
import { ReactNode } from 'react'
import {
  Control,
  ControllerFieldState,
  ControllerProps,
  ControllerRenderProps,
  FieldPath,
  FieldValues,
  UseFormStateReturn,
} from 'react-hook-form'

import { Flex } from '@ui/components/ui'
import { cn } from '@ui/lib'

import { Label } from '../ui/label'

import { FormControl, FormDescription, FormField, FormItem, FormLabel, FormMessage } from './form'

const formFieldVariants = cva('', {
  variants: {
    layout: {
      vertical: '',
      horizontal: 'space-y-0 grid gap-2 md:grid md:grid-cols-12 items-center',
    },
  },
})
const formFieldLabelVariants = cva('dark:text-muted-foreground font-normal text-xs', {
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
  required?: boolean
  key?: string
  containerClassName?: string
  labelClassName?: string
  className?: string
  description?: string
  layout?: 'vertical' | 'horizontal' | null
  rightLabel?: ReactNode
  labelTooltip?: ReactNode
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

/**
 * A field's label row above its control, as every form field draws it: 12px and quiet, 8px over
 * the control and 8px under whatever is above, so fields side by side line up.
 *
 * Inside a FormField pass `asFormLabel`: the label then points at the field and turns red on an
 * error. Without it, it labels a control outside a form (a read-only select, a copy button); give
 * it the control's id in `htmlFor`.
 */
export const FieldLabel = ({
  children,
  htmlFor,
  required,
  asFormLabel = false,
  layout = 'vertical',
  labelClassName,
  tooltip,
  rightLabel,
}: {
  children: ReactNode
  htmlFor?: string
  required?: boolean
  asFormLabel?: boolean
  layout?: 'vertical' | 'horizontal' | null
  labelClassName?: string
  tooltip?: ReactNode
  rightLabel?: ReactNode
}) => {
  const labelProps = {
    className: cn(
      !asFormLabel && 'text-foreground/60',
      formFieldLabelVariants({ layout }),
      labelClassName
    ),
    // FormLabel sets its own htmlFor; an explicit undefined would clear it.
    ...(htmlFor ? { htmlFor } : {}),
  }
  const content = (
    <>
      {children}
      {required ? <span className="text-destructive text-xs pl-1">*</span> : null}
    </>
  )

  return (
    <Flex
      align="center"
      justify="between"
      className={cn('my-2', formFieldLabelVariants({ layout }), labelClassName)}
    >
      <div className="flex items-center gap-1">
        {asFormLabel ? (
          <FormLabel {...labelProps}>{content}</FormLabel>
        ) : (
          <Label {...labelProps}>{content}</Label>
        )}
        {tooltip}
      </div>
      {rightLabel}
    </Flex>
  )
}

export const GenericFormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  render,
  label,
  required,
  layout = 'vertical',
  containerClassName,
  labelClassName,
  className,
  description,
  rightLabel,
  labelTooltip,
  ...props
}: GenericFormFieldProps<TFieldValues, TName>) => {
  return (
    <FormField
      {...props}
      render={fieldProps => (
        <FormItem className={cn(formFieldVariants({ layout }), containerClassName)}>
          {label && (
            <FieldLabel
              asFormLabel
              required={required}
              layout={layout}
              labelClassName={labelClassName}
              tooltip={labelTooltip}
              rightLabel={rightLabel}
            >
              {label}
            </FieldLabel>
          )}
          <FormControl>
            {render({ ...fieldProps, className: cn(inputVariants({ layout }), className) })}
          </FormControl>
          {description && (
            <FormDescription className={formFieldMessageVariants({ layout })}>
              {description}
            </FormDescription>
          )}
          <FormMessage className={formFieldMessageVariants({ layout })} />
        </FormItem>
      )}
    />
  )
}
