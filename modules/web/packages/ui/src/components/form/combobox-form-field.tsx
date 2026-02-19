import { CaretSortIcon, CheckIcon } from '@radix-ui/react-icons'
import { useState } from 'react'
import { Control, ControllerProps, FieldPath, FieldValues } from 'react-hook-form'

import { cn } from '@ui/lib'

import { Button } from '../ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from '../ui/command'
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover'

import { FormControl, FormDescription, FormField, FormItem, FormLabel, FormMessage } from './form'
import { genericFormFieldVariants as genVariants } from './generic-form-field'

interface FormComboboxProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<ControllerProps<TFieldValues, TName>, 'render'> {
  control: Control<TFieldValues>
  options: { label: React.ReactNode; value: string; keywords?: string[] }[]
  label?: string
  description?: string
  containerClassName?: string
  labelClassName?: string
  className?: string
  layout?: 'vertical' | 'horizontal' | null
  hasSearch?: boolean
  placeholder?: string
  action?: React.ReactNode
  unit?: string
}
export function ComboboxFormField<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
>({
  options,
  label,
  description,
  layout = 'vertical',
  containerClassName,
  labelClassName,
  className,
  hasSearch,
  placeholder,
  action,
  unit = '...',
  ...props
}: FormComboboxProps<TFieldValues, TName>) {
  const [open, setOpen] = useState(false)

  return (
    <FormField
      {...props}
      render={({ field }) => (
        <FormItem className={cn(genVariants.formField({ layout }), containerClassName)}>
          {label && (
            <FormLabel className={cn('my-2', genVariants.label({ layout }), labelClassName)}>
              {label}
            </FormLabel>
          )}
          <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>
              <FormControl>
                <Button
                  variant="outline"
                  role="combobox"
                  aria-expanded={open}
                  className={cn(
                    'flex h-9 w-full border border-border items-center justify-between whitespace-nowrap rounded-md font-normal  bg-transparent hover:bg-transparent px-3 py-2 text-sm shadow-sm ring-offset-background placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1',
                    className
                    //!field.value && ''
                  )}
                >
                  {field.value
                    ? options.find(option => option.value === field.value)?.label
                    : (placeholder ?? `Select ${unit}`)}
                  <CaretSortIcon className="ml-2 h-4 w-4 shrink-0 opacity-50" />
                </Button>
              </FormControl>
            </PopoverTrigger>
            <PopoverContent className="w-[var(--radix-popover-trigger-width)] p-0">
              <Command className="border border-border ">
                {hasSearch && (
                  <>
                    <CommandInput placeholder={`Search ${unit}`} className="h-9  " />
                    <CommandEmpty>No data found.</CommandEmpty>
                  </>
                )}

                <CommandList>
                  {options.map((option, index) => (
                    <CommandItem
                      value={`${option.value}/${option.keywords?.join(' ')}`}
                      key={option.value}
                      keywords={option.keywords}
                      autoFocus={index === 0}
                      onSelect={() => {
                        field.onChange(option.value)
                        setOpen(false)
                      }}
                    >
                      {option.label}
                      <CheckIcon
                        className={cn(
                          'ml-auto h-4 w-4',
                          option.value === field.value ? 'opacity-100' : 'opacity-0'
                        )}
                      />
                    </CommandItem>
                  ))}
                  {!options.length && (
                    <CommandGroup>
                      <CommandItem disabled>No data.</CommandItem>
                    </CommandGroup>
                  )}
                  {action && (
                    <>
                      <CommandSeparator />
                      <div
                        className="h-8 relative flex cursor-default select-none items-center rounded-sm pt-1 text-sm outline-none"
                        onClick={() => setOpen(false)}
                      >
                        {action}
                      </div>
                    </>
                  )}
                </CommandList>
              </Command>
            </PopoverContent>
          </Popover>
          {description && <FormDescription>{description}</FormDescription>}

          <FormMessage className={genVariants.message({ layout })} />
        </FormItem>
      )}
    />
  )
}
