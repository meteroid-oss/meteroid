'use client'

import {
  ButtonLegacy as Button,
  Calendar,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@ui/components'
import { cn } from '@ui/lib'
import { format } from 'date-fns'
import { Calendar as CalendarIcon } from 'lucide-react'
import * as React from 'react'
import { DateRange } from 'react-day-picker'


export interface DatePickerWithRangeProps {
  range: DateRange | undefined
  setRange: (range: DateRange | undefined) => void
}
export function DatePickerWithRange({
  className,
  range,
  setRange,
}: React.HTMLAttributes<HTMLDivElement> & DatePickerWithRangeProps) {
  return (
    <div className={cn('grid gap-2', className)}>
      <Popover>
        <PopoverTrigger asChild>
          <Button
            id="date"
            variant="outline"
            className={cn(
              'w-[250px] justify-start text-left font-normal border border-slate-400 rounded-md',
              !range && 'text-muted-foreground'
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
        <PopoverContent className="w-auto p-0 border border-slate-400 rounded-md " align="start">
          <Calendar
            initialFocus
            mode="range"
            defaultMonth={range?.from}
            selected={range}
            onSelect={setRange}
            numberOfMonths={2}
            className="bg-white-100 dark:bg-slate-200"
          />
        </PopoverContent>
      </Popover>
    </div>
  )
}
