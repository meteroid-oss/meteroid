import { Control, FieldPath, FieldValues, UseControllerProps } from 'react-hook-form'

import {
  MultiSelect,
  MultiSelectContent,
  MultiSelectEmpty,
  MultiSelectList,
  MultiSelectSearch,
  MultiSelectTrigger,
  MultiSelectValue,
  type MultiSelectProps,
} from '@ui/components/ui/multi-select'
import { cn } from '@ui/lib'

import { GenericFormField, GenericFormFieldVariantProps } from './generic-form-field'

type MultiSelectFormFieldProps<T extends FieldValues, TName extends FieldPath<T>> = Omit<
  MultiSelectProps,
  'defaultValue' | 'name'
> &
  UseControllerProps<T, TName> & {
    label?: string
    key?: string
    className?: string
    description?: string
    containerClassName?: string
    labelClassName?: string
    labelTooltip?: React.ReactNode
    contentClassName?: string
    placeholder?: string
    empty?: boolean
    hasSearch?: boolean
    control: Control<T>
  }

export const MultiSelectFormField = <
  TFieldValues extends FieldValues = FieldValues,
  TName extends FieldPath<TFieldValues> = FieldPath<TFieldValues>,
>({
  children,
  empty,
  hasSearch,
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
  description,
  placeholder,
  contentClassName,
}: MultiSelectFormFieldProps<TFieldValues, TName> & GenericFormFieldVariantProps) => {
  return (
    <GenericFormField
      key={key}
      label={label}
      description={description}
      name={name}
      rules={rules}
      defaultValue={defaultValue}
      control={control}
      disabled={disabled}
      shouldUnregister={shouldUnregister}
      layout={layout}
      labelClassName={labelClassName}
      containerClassName={containerClassName}
      className={cn('flex-row', className)}
      render={({ field, className }) => {
        return (
          <MultiSelect onValueChange={field.onChange} value={field.value} disabled={disabled}>
            <MultiSelectTrigger ref={field.ref} className={className}>
              <MultiSelectValue placeholder={placeholder} />
            </MultiSelectTrigger>
            <MultiSelectContent className={contentClassName} hideWhenDetached>
              <MultiSelectList>
                {hasSearch && <MultiSelectSearch placeholder="Input to search" />}
                {empty && <MultiSelectEmpty />}
                {children}
              </MultiSelectList>
            </MultiSelectContent>
          </MultiSelect>
        )
      }}
    />
  )
}
