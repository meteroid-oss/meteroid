import { ChevronLeftIcon, ChevronRightIcon } from '@radix-ui/react-icons'
import * as React from 'react'
import { DayPicker, type ChevronProps, type DropdownProps } from 'react-day-picker'

import { buttonVariants } from '@ui/components/ui/button'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@ui/components/ui/select'
import { cn } from '@ui/lib'

export type CalendarProps = React.ComponentProps<typeof DayPicker>

// react-day-picker renders month/year navigation as native <select> elements.
// We swap in our own Select so the dropdowns match the rest of the app.
function CalendarDropdown({ value, onChange, options = [] }: DropdownProps) {
  const selected = options.find(option => String(option.value) === String(value))

  const handleChange = (next: string) => {
    onChange?.({ target: { value: next } } as React.ChangeEvent<HTMLSelectElement>)
  }

  return (
    <Select value={value?.toString()} onValueChange={handleChange}>
      <SelectTrigger className="h-7 w-fit gap-1 border-none px-2 py-0 font-medium shadow-none focus:ring-0 focus:ring-offset-0 hover:bg-accent">
        <SelectValue>{selected?.label}</SelectValue>
      </SelectTrigger>
      <SelectContent position="popper" className="max-h-64">
        {options.map(option => (
          <SelectItem
            key={option.value}
            value={String(option.value)}
            disabled={option.disabled}
          >
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function CalendarChevron({ orientation }: ChevronProps) {
  return orientation === 'left' ? (
    <ChevronLeftIcon className="h-4 w-4" />
  ) : (
    <ChevronRightIcon className="h-4 w-4" />
  )
}

function Calendar({ className, classNames, showOutsideDays = true, ...props }: CalendarProps) {
  const brandButton =
    '[&>button]:bg-brand [&>button]:text-brand-foreground [&>button]:hover:bg-brand [&>button]:hover:text-brand-foreground [&>button]:focus:bg-brand [&>button]:focus:text-brand-foreground'
  // An endpoint shown in a neighbouring month stays muted, as in the month it belongs to.
  const rangeEndButton =
    '[&:not(.day-outside)>button]:bg-brand [&:not(.day-outside)>button]:text-brand-foreground [&:not(.day-outside)>button]:hover:bg-brand [&:not(.day-outside)>button]:hover:text-brand-foreground [&:not(.day-outside)>button]:focus:bg-brand [&:not(.day-outside)>button]:focus:text-brand-foreground'
  const navButton = cn(
    buttonVariants({ variant: 'outline' }),
    'absolute top-0 z-10 h-7 w-7 bg-transparent p-0 opacity-50 hover:opacity-100'
  )

  return (
    <DayPicker
      showOutsideDays={showOutsideDays}
      captionLayout="dropdown"
      startMonth={new Date(2000, 0)}
      endMonth={new Date(new Date().getFullYear() + 10, 11)}
      className={cn('p-3', className)}
      classNames={{
        months: 'relative flex flex-col sm:flex-row gap-4',
        month: 'space-y-4',
        month_caption: 'flex justify-center h-7 relative items-center',
        caption_label: 'text-sm font-medium',
        dropdowns: 'flex justify-center gap-1',
        nav: 'absolute inset-x-0 top-0',
        button_previous: cn(navButton, 'left-1'),
        button_next: cn(navButton, 'right-1'),
        month_grid: 'w-full border-collapse space-y-1',
        weekdays: 'flex',
        weekday: 'text-muted-foreground rounded-md w-8 font-normal text-[0.8rem]',
        week: 'flex w-full mt-2',
        day: cn(
          'relative p-0 text-center text-sm focus-within:relative focus-within:z-20 [&[aria-selected]]:bg-accent [&[aria-selected].day-outside]:bg-accent/50',
          props.mode === 'range'
            ? '[&.day-range-end]:rounded-r-md [&.day-range-start]:rounded-l-md first:[&[aria-selected]]:rounded-l-md last:[&[aria-selected]]:rounded-r-md'
            : '[&[aria-selected]]:rounded-md'
        ),
        day_button: cn(
          buttonVariants({ variant: 'ghost' }),
          'h-8 w-8 p-0 font-normal aria-selected:opacity-100'
        ),
        range_start: cn('day-range-start', rangeEndButton),
        range_end: cn('day-range-end', rangeEndButton),
        selected: props.mode === 'range' ? undefined : brandButton,
        today: '[&>button]:bg-accent [&>button]:text-accent-foreground',
        outside:
          'day-outside text-muted-foreground opacity-50 aria-selected:bg-accent/50 aria-selected:text-muted-foreground aria-selected:opacity-30',
        disabled: 'text-muted-foreground opacity-50',
        range_middle: '[&>button]:text-accent-foreground',
        hidden: 'invisible',
        ...classNames,
      }}
      components={{
        Dropdown: CalendarDropdown,
        Chevron: CalendarChevron,
      }}
      {...props}
    />
  )
}
Calendar.displayName = 'Calendar'

export { Calendar }
