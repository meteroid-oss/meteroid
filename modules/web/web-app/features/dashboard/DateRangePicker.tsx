'use client'

import { Button, Calendar, Popover, PopoverContent, PopoverTrigger } from '@md/ui'
import { cn } from '@md/ui'
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
              'w-[150px] md:w-[250px] justify-start text-left font-normal rounded-md overflow-hidden',
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
        <PopoverContent className="w-auto rounded-md " align="start">
          <Calendar
            initialFocus
            mode="range"
            defaultMonth={range?.from}
            selected={range}
            onSelect={setRange}
            numberOfMonths={2}
          />
        </PopoverContent>
      </Popover>
    </div>
  )
}
