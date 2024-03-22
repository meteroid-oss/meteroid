import { format } from 'date-fns'
import { Calendar as CalendarIcon } from 'lucide-react'
import { ComponentProps } from 'react'
import { DayPicker } from 'react-day-picker'

import { Button, Calendar, Popover, PopoverContent, PopoverTrigger } from '@ui/components'
import { cn } from '@ui/lib'

export interface Props {
  placeholder?: string
  date?: Date
}

function DatePicker({
  placeholder = 'Pick a date',
  date,
  className,
  ...props
}: Props & ComponentProps<typeof DayPicker>) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          className={cn(
            'justify-start text-left font-normal border border-slate-400 rounded-md',
            !date && 'text-muted-foreground',
            className
          )}
        >
          <CalendarIcon className="mr-2 h-4 w-4" />
          {date ? format(date, 'LLL dd, y') : <span>{placeholder}</span>}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-auto p-0 border border-slate-400 rounded-md " align="start">
        <Calendar initialFocus className="bg-white-100 dark:bg-slate-200" {...props} />
      </PopoverContent>
    </Popover>
  )
}
DatePicker.displayName = 'DatePicker'

export { DatePicker }
