'use client'

import { Button, Calendar, Popover, PopoverContent, PopoverTrigger, cn } from '@md/ui'
import { addMonths, format, isSameDay, isSameMonth, startOfMonth } from 'date-fns'
import { Calendar as CalendarIcon } from 'lucide-react'
import * as React from 'react'
import { DateRange } from 'react-day-picker'

export interface DateRangePickerPreset {
  label: string
  getRange: () => DateRange
}

export interface DatePickerWithRangeProps {
  range: DateRange | undefined
  setRange: (range: DateRange | undefined) => void
  disabled?: boolean
  presets?: DateRangePickerPreset[]
  /** Extra classes for the trigger button, e.g. to match a compact toolbar height. */
  triggerClassName?: string
}
export function DatePickerWithRange({
  className,
  range,
  setRange,
  disabled,
  presets,
  triggerClassName,
}: React.HTMLAttributes<HTMLDivElement> & DatePickerWithRangeProps) {
  const [open, setOpen] = React.useState(false)

  // Two-month layout renders two independently-navigable calendars sharing one
  // range, so start and end months that are far apart (e.g. Jun 2025 → Jun 2026)
  // can be viewed and picked at once — react-day-picker's own numberOfMonths={2}
  // locks the panes to consecutive months.
  const twoMonths = Boolean(presets?.length)
  const [leftMonth, setLeftMonth] = React.useState<Date | undefined>()
  const [rightMonth, setRightMonth] = React.useState<Date | undefined>()

  const handleOpenChange = (next: boolean) => {
    if (next) {
      const from = range?.from ?? new Date()
      const to = range?.to
      setLeftMonth(startOfMonth(from))
      setRightMonth(startOfMonth(to && !isSameMonth(to, from) ? to : addMonths(from, 1)))
    }
    setOpen(next)
  }

  const isPresetActive = (preset: DateRangePickerPreset) => {
    const r = preset.getRange()
    return (
      range?.from &&
      range?.to &&
      r.from &&
      r.to &&
      isSameDay(range.from, r.from) &&
      isSameDay(range.to, r.to)
    )
  }

  return (
    <div className={cn('grid gap-2', className)}>
      <Popover open={open} onOpenChange={handleOpenChange}>
        <PopoverTrigger asChild>
          <Button
            id="date"
            variant="outline"
            disabled={disabled}
            className={cn(
              'w-[150px] md:w-[250px] justify-start text-left font-normal rounded-md overflow-hidden',
              !range && 'text-muted-foreground',
              triggerClassName
            )}
          >
            <CalendarIcon className="mr-2 h-4 w-4" />
            {range?.from ? (
              range.to ? (
                <>
                  {format(range.from, 'LLL dd, y')} - {format(range.to, 'LLL dd, y')}
                </>
              ) : (
                format(range.from, 'LLL dd, y')
              )
            ) : (
              <span>Pick a date</span>
            )}
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-auto rounded-md p-0" align="start">
          <div className="flex">
            {presets?.length ? (
              <div className="flex w-[150px] flex-col gap-0.5 border-r border-border p-2">
                {presets.map(preset => (
                  <Button
                    key={preset.label}
                    variant="ghost"
                    className={cn(
                      'h-8 justify-start px-2 font-normal',
                      isPresetActive(preset) && 'bg-accent text-accent-foreground'
                    )}
                    onClick={() => {
                      setRange(preset.getRange())
                      setOpen(false)
                    }}
                  >
                    {preset.label}
                  </Button>
                ))}
              </div>
            ) : null}
            {twoMonths ? (
              <div className="flex">
                <Calendar
                  initialFocus
                  mode="range"
                  month={leftMonth}
                  onMonthChange={setLeftMonth}
                  selected={range}
                  onSelect={setRange}
                  numberOfMonths={1}
                  disabled={disabled}
                />
                <Calendar
                  mode="range"
                  month={rightMonth}
                  onMonthChange={setRightMonth}
                  selected={range}
                  onSelect={setRange}
                  numberOfMonths={1}
                  disabled={disabled}
                />
              </div>
            ) : (
              <Calendar
                initialFocus
                mode="range"
                defaultMonth={range?.from}
                selected={range}
                onSelect={setRange}
                numberOfMonths={1}
                disabled={disabled}
              />
            )}
          </div>
        </PopoverContent>
      </Popover>
    </div>
  )
}
