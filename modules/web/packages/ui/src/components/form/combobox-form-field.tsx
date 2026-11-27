import { CaretSortIcon, CheckIcon, Cross2Icon } from '@radix-ui/react-icons'
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

import { FormControl, FormDescription, FormField, FormItem, FormMessage } from './form'
import { FieldLabel, genericFormFieldVariants as genVariants } from './generic-form-field'

interface FormComboboxProps<
  TFieldValues extends FieldValues,
  TName extends FieldPath<TFieldValues>,
> extends Omit<ControllerProps<TFieldValues, TName>, 'render'> {
  control: Control<TFieldValues>
  options: { label: React.ReactNode; value: string; keywords?: string[] }[]
  required?: boolean
  label?: string
  description?: string
  containerClassName?: string
  labelClassName?: string
  className?: string
  layout?: 'vertical' | 'horizontal' | null
  hasSearch?: boolean
  /** Show a clear (×) affordance to reset the field to an empty value. */
  clearable?: boolean
  /** Called every time the search input changes — use it to drive server-side search. */
  onSearchChange?: (value: string) => void
  /** Set false when the caller filters `options` itself (e.g. server-side search). */
  shouldFilter?: boolean
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
  required,
  description,
  layout = 'vertical',
  containerClassName,
  labelClassName,
  className,
  hasSearch,
  clearable,
  onSearchChange,
  shouldFilter,
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
          {/* The same label row as every other field: on the bare FormLabel (inline), the
              vertical margin did nothing and the label sat on the control. */}
          {label && (
            <FieldLabel
              asFormLabel
              required={required}
              layout={layout}
              labelClassName={labelClassName}
            >
              {label}
            </FieldLabel>
          )}
          <Popover open={open} onOpenChange={setOpen} modal>
            <PopoverTrigger asChild>
              <FormControl>
                <Button
                  variant="outline"
                  role="combobox"
                  aria-expanded={open}
                  aria-required={required}
                  className={cn(
                    // Drawn as a text field (see Input).
                    'flex h-8 w-full items-center justify-between whitespace-nowrap rounded-md border border-field-border bg-field px-3 py-1 text-[13px] font-normal text-field-foreground shadow-none transition-[border-color,box-shadow] hover:bg-field placeholder:text-field-placeholder focus:outline-none focus-visible:border-foreground/25 focus-visible:ring-[3px] focus-visible:ring-foreground/8 disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1',
                    // horizontal layout makes FormItem a 12-col grid — claim the 8 columns
                    // the label leaves, like SelectFormField does. flex-row undoes the
                    // variant's flex-col, which would stack the value above the caret.
                    genVariants.input({ layout }),
                    'flex-row',
                    className
                    //!field.value && ''
                  )}
                >
                  <span className="truncate text-left">
                    {field.value
                      ? options.find(option => option.value === field.value)?.label
                      : (placeholder ?? `Select ${unit}`)}
                  </span>
                  <div className="ml-2 flex shrink-0 items-center gap-1">
                    {clearable && field.value ? (
                      <span
                        role="button"
                        tabIndex={-1}
                        aria-label="Clear"
                        className="flex items-center justify-center rounded-sm opacity-60 hover:opacity-100"
                        onPointerDown={e => {
                          e.preventDefault()
                          e.stopPropagation()
                          field.onChange('')
                        }}
                      >
                        <Cross2Icon className="h-3.5 w-3.5" />
                      </span>
                    ) : null}
                    <CaretSortIcon className="h-4 w-4 opacity-50" />
                  </div>
                </Button>
              </FormControl>
            </PopoverTrigger>
            <PopoverContent className="w-[var(--radix-popover-trigger-width)] p-0">
              <Command className="border border-border " shouldFilter={shouldFilter}>
                {hasSearch && (
                  <>
                    <CommandInput
                      placeholder={`Search ${unit}`}
                      className="h-9  "
                      onValueChange={onSearchChange}
                    />
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
                        className="h-8 relative flex cursor-pointer select-none items-center rounded-sm pt-1 text-sm outline-none"
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
