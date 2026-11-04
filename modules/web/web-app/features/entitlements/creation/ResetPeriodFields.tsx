import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { InfoIcon } from 'lucide-react'
import { useFormContext, useWatch } from 'react-hook-form'

import { CalendarUnit } from '@/rpc/api/entitlements/v1/models_pb'

import type { ResetPeriodType } from './types'

const Info = ({ text }: { text: string }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <InfoIcon size={12} className="text-muted-foreground cursor-help" />
    </TooltipTrigger>
    <TooltipContent className="max-w-56">{text}</TooltipContent>
  </Tooltip>
)

const Req = () => <span className="text-destructive ml-0.5">*</span>

const resetPeriodHint = (type: string | undefined): string => {
  switch (type) {
    case 'never':
      return 'Never resets — counts all usage since the subscription was activated.'
    case 'billingCycle':
      return 'Resets each time the subscription renews — anchored to its billing cycle.'
    case 'calendar':
      return 'Resets on calendar boundaries in UTC (e.g. the 1st of every month) — not tied to any subscription date.'
    case 'fixedWindow':
      return 'Resets at regular intervals, counted from the exact activation time of the subscription.'
    case 'slidingWindow':
      return 'Always ends at now — e.g. 30 days means the last 30 days, old usage drops off automatically. It sends no threshold or limit webhooks.'
    default:
      return ''
  }
}

const PERIOD_OPTIONS: { value: ResetPeriodType; label: string }[] = [
  { value: 'never', label: 'Never (lifetime cap)' },
  { value: 'billingCycle', label: 'Billing cycle' },
  { value: 'calendar', label: 'Calendar' },
  { value: 'fixedWindow', label: 'Fixed window' },
  { value: 'slidingWindow', label: 'Sliding window' },
]

/** Reads and writes `resetPeriodType`, `resetInterval` and `resetUnit` of the surrounding form. */
export const ResetPeriodFields = ({
  excludeSliding,
}: {
  excludeSliding?: boolean
}) => {
  const { control } = useFormContext()
  const resetPeriodType = useWatch({ control, name: 'resetPeriodType' })
  const showInterval =
    resetPeriodType === 'calendar' ||
    resetPeriodType === 'fixedWindow' ||
    resetPeriodType === 'slidingWindow'

  return (
    <>
      <FormField
        control={control}
        name="resetPeriodType"
        render={({ field }) => (
          <FormItem>
            <FormLabel className="flex items-center gap-1">
              Reset period
              <Req />
            </FormLabel>
            <Select value={field.value} onValueChange={field.onChange}>
              <FormControl>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                {PERIOD_OPTIONS.filter(o => !excludeSliding || o.value !== 'slidingWindow').map(
                  option => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  )
                )}
              </SelectContent>
            </Select>
            {field.value && (
              <p className="text-xs text-muted-foreground mt-1">{resetPeriodHint(field.value)}</p>
            )}
          </FormItem>
        )}
      />
      {showInterval && (
        <div className="flex gap-3">
          <FormField
            control={control}
            name="resetInterval"
            render={({ field }) => (
              <FormItem className="flex-1">
                <FormLabel className="flex items-center gap-1">
                  Every
                  <Req />
                  <Info text="Number of calendar units between resets (e.g. 2 = every 2 months)." />
                </FormLabel>
                <FormControl>
                  <Input type="number" min={1} {...field} />
                </FormControl>
              </FormItem>
            )}
          />
          <FormField
            control={control}
            name="resetUnit"
            render={({ field }) => (
              <FormItem className="flex-1">
                <FormLabel className="flex items-center gap-1">
                  Unit
                  <Req />
                  <Info text="Calendar unit for the reset interval (e.g. day, week, month)." />
                </FormLabel>
                <Select value={String(field.value)} onValueChange={v => field.onChange(Number(v))}>
                  <FormControl>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    <SelectItem value={String(CalendarUnit.HOUR)}>Hours</SelectItem>
                    <SelectItem value={String(CalendarUnit.DAY)}>Days</SelectItem>
                    <SelectItem value={String(CalendarUnit.WEEK)}>Weeks</SelectItem>
                    <SelectItem value={String(CalendarUnit.MONTH)}>Months</SelectItem>
                    <SelectItem value={String(CalendarUnit.YEAR)}>Years</SelectItem>
                  </SelectContent>
                </Select>
              </FormItem>
            )}
          />
        </div>
      )}
    </>
  )
}
