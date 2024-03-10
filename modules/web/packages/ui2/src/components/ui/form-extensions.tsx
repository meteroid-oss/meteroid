import {
  FieldValues,
  FieldPath,
  RegisterOptions,
  FieldPathValue,
  Control,
  UseControllerProps,
} from 'react-hook-form'
import { FormItem, FormField, FormLabel, FormControl, FormMessage } from './form'
import { Input as BaseInput } from './input'
import { Checkbox as BaseCheckbox } from './checkbox'
import { VariantProps, cva } from 'class-variance-authority'
import { cn } from '../../lib'

interface InputFieldProps<TFieldValues extends FieldValues, TName extends FieldPath<TFieldValues>>
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, 'defaultValue' | 'name'>,
    UseControllerProps<TFieldValues, TName> {
  label: string
  key?: string
  containerClassName?: string
  labelClassName?: string
}

const inputVariants = cva('', {
  variants: {
    direction: {
      vertical: '',
      horizontal: 'space-y-0 grid gap-2 md:grid md:grid-cols-12',
    },
  },
})
const inputLabelVariants = cva('', {
  variants: {
    direction: {
      vertical: '',
      horizontal: 'col-span-4 text-muted-foreground ',
    },
  },
}) satisfies typeof inputVariants

const inputInputVariants = cva('', {
  variants: {
    direction: {
      vertical: '',
      horizontal: 'col-span-8 flex flex-col gap-2',
    },
  },
}) satisfies typeof inputVariants

const inputMessageVariants = cva('', {
  variants: {
    direction: {
      vertical: '',
      horizontal: 'col-span-12',
    },
  },
}) satisfies typeof inputVariants

const parseNumber = (value?: string) => (Number.isNaN(Number(value)) ? undefined : Number(value))

const Input = <
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
  direction = 'vertical',
  containerClassName,
  labelClassName,
  className,
  ...props
}: InputFieldProps<TFieldValues, TName> & VariantProps<typeof inputVariants>) => {
  return (
    <FormField
      control={control}
      name={name}
      rules={rules}
      defaultValue={defaultValue}
      disabled={disabled}
      key={key}
      shouldUnregister={shouldUnregister}
      render={({ field }) => {
        const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
          // valueAsNumber is not available with controller
          if (props.type == 'number') {
            const newValue = parseNumber(e.target.value)
            field.onChange({ ...e, target: { ...e.target, value: newValue } })
          } else {
            field.onChange(e)
          }
        }

        return (
          <FormItem className={cn(inputVariants({ direction }), containerClassName)}>
            <FormLabel className={cn(inputLabelVariants({ direction }), labelClassName)}>
              {label}
            </FormLabel>
            <FormControl>
              <BaseInput
                {...field}
                {...props}
                onChange={onChange}
                className={cn(inputInputVariants({ direction }), className)}
              />
            </FormControl>
            <FormMessage className={inputMessageVariants({ direction })} />
          </FormItem>
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

export { Input as FormInput, Checkbox as FormCheckbox }
