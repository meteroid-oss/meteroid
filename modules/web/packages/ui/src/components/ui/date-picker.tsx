import { format, isSameDay } from 'date-fns'
import { Calendar as CalendarIcon, X } from 'lucide-react'
import { useState } from 'react'

import { Calendar, Popover, PopoverContent, PopoverTrigger } from '@ui/components'
import { cn } from '@ui/lib'

import type { PropsBase, PropsSingle } from 'react-day-picker'

/** A ready-made choice beside the calendar, e.g. "In 30 days". */
export interface DatePreset {
  label: string
  date: Date
}

export interface Props {
  placeholder?: string
  date?: Date
  clearable?: boolean
  /** Quick choices listed beside the calendar; picking one sets the date and closes. */
  presets?: DatePreset[]
}

function DatePicker({
  placeholder = 'Pick a date',
  date,
  className,
  clearable = true,
  presets,
  ...props
}: Props & PropsBase & PropsSingle) {
  const [open, setOpen] = useState(false)

  const handleClear = (e: React.MouseEvent) => {
    e.stopPropagation()
    props.onSelect?.(undefined, new Date(), {}, e)
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        {/* Drawn as a text field (see Input): the same fill, border, height, type and focus halo. */}
        <button
          type="button"
          className={cn(
            'flex h-8 w-full items-center gap-2 rounded-md border border-field-border bg-field px-3 text-left text-[13px] text-field-foreground transition-[color,border-color,box-shadow]',
            'focus-visible:outline-none focus-visible:border-foreground/25 focus-visible:ring-[3px] focus-visible:ring-foreground/8',
            'disabled:cursor-not-allowed disabled:opacity-50',
            className
          )}
        >
          <CalendarIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden />
          {date ? (
            <span className="truncate">{format(date, 'LLL dd, y')}</span>
          ) : (
            <span className="truncate text-field-placeholder">{placeholder}</span>
          )}
          {clearable && date && (
            <X
              className="ml-auto h-3.5 w-3.5 shrink-0 text-muted-foreground hover:text-foreground"
              onClick={handleClear}
              aria-label="Clear date"
            />
          )}
        </button>
      </PopoverTrigger>
      {/* Edge and divider in white at 10.2%, as the other menus: never the solid dark border. */}
      <PopoverContent className="flex w-auto border-foreground/[0.102] p-0" align="start">
        {presets && presets.length > 0 && (
          <div className="flex w-44 flex-col gap-0.5 border-r-[0.5px] border-foreground/[0.102] p-1.5">
            {presets.map(preset => {
              const chosen = date !== undefined && isSameDay(date, preset.date)
              return (
                <button
                  key={preset.label}
                  type="button"
                  aria-pressed={chosen}
                  onClick={e => {
                    props.onSelect?.(preset.date, preset.date, {}, e)
                    setOpen(false)
                  }}
                  className={cn(
                    'flex items-baseline justify-between gap-2 rounded-[5px] px-2 py-1.5 text-left text-[13px] transition-colors hover:bg-foreground/[0.06] focus-visible:outline-none focus-visible:bg-foreground/[0.06]',
                    chosen ? 'bg-foreground/[0.08] text-foreground' : 'text-foreground/80'
                  )}
                >
                  <span>{preset.label}</span>
                  <span className="text-[11px] tabular-nums text-muted-foreground">
                    {format(preset.date, 'MMM d')}
                  </span>
                </button>
              )
            })}
          </div>
        )}
        <Calendar autoFocus={!presets?.length} className=" " selected={date} {...props} />
      </PopoverContent>
    </Popover>
  )
}
DatePicker.displayName = 'DatePicker'

export { DatePicker }
