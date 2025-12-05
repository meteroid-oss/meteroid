import { format } from 'date-fns'
import { Calendar as CalendarIcon, X } from 'lucide-react'
import { DayPickerSingleProps } from 'react-day-picker'

import { Button, Calendar, Popover, PopoverContent, PopoverTrigger } from '@ui/components'
import { cn } from '@ui/lib'

export interface Props {
  placeholder?: string
  date?: Date
  clearable?: boolean
}

function DatePicker({
  placeholder = 'Pick a date',
  date,
  className,
  clearable = true,
  ...props
}: Props & DayPickerSingleProps) {
  const handleClear = (e: React.MouseEvent) => {
    e.stopPropagation()
    props.onSelect?.(undefined, new Date(), {}, e)
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          className={cn(
            'justify-start text-left font-normal border border-border rounded-md',
            !date && 'text-muted-foreground',
            className
          )}
        >
          <CalendarIcon className="mr-2 h-4 w-4" />
          {date ? format(date, 'LLL dd, y') : <span>{placeholder}</span>}
          {clearable && date && (
            <X
              className="ml-auto h-4 w-4 text-muted-foreground hover:text-foreground"
              onClick={handleClear}
            />
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-auto p-0 border border-border rounded-md " align="start">
        <Calendar initialFocus className=" " selected={date} {...props} />
      </PopoverContent>
    </Popover>
  )
}
DatePicker.displayName = 'DatePicker'

export { DatePicker }
