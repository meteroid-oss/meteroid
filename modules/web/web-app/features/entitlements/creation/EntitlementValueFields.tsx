import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Label,
  RadioGroup,
  RadioGroupItem,
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
import { Fragment } from 'react'
import { useFormContext, useWatch } from 'react-hook-form'

import { CalendarUnit } from '@/rpc/api/entitlements/v1/models_pb'

interface Props {
  featureType: 'boolean' | 'metered'
  idPrefix: string // makes radio IDs unique when multiple instances exist on the same page
}

function Info({ text }: { text: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <InfoIcon size={12} className="text-muted-foreground cursor-help" />
      </TooltipTrigger>
      <TooltipContent className="max-w-56">{text}</TooltipContent>
    </Tooltip>
  )
}

function Req() {
  return <span className="text-destructive ml-0.5">*</span>
}

function Opt() {
  return <span className="text-muted-foreground text-xs ml-1">(optional)</span>
}

function overageBehaviorHint(type: string | undefined): string {
  switch (type) {
    case 'block':
      return 'Deny access once usage reaches the limit. Meteroid does not enforce this — your integration must check and act.'
    case 'allow':
      return 'Keep serving usage past the limit; overage is billed or handled out-of-band.'
    default:
      return ''
  }
}

function resetPeriodHint(type: string | undefined): string {
  switch (type) {
    case 'never':
      return 'Usage accumulates for the life of the subscription — no reset.'
    case 'billingCycle':
      return 'Resets at the start of each subscription billing period.'
    case 'calendar':
      return 'Buckets aligned to the calendar (e.g. every 1st of the month), shared across customers.'
    case 'fixedWindow':
      return 'Fixed-length buckets anchored on the subscription activation date. Resets at every boundary.'
    case 'slidingWindow':
      return 'Continuous rolling window — usage older than the window edge drops out, no fixed reset.'
    default:
      return ''
  }
}

export function EntitlementValueFields({ featureType, idPrefix }: Props) {
  const { control } = useFormContext()
  const resetPeriodType = useWatch({ control, name: 'resetPeriodType' })
  const overageBehaviorType = useWatch({ control, name: 'overageBehaviorType' })
  const showInterval =
    resetPeriodType === 'calendar' ||
    resetPeriodType === 'fixedWindow' ||
    resetPeriodType === 'slidingWindow'
  const showGracePeriod = overageBehaviorType === 'block'

  if (featureType === 'boolean') {
    return (
      <Fragment key="boolean">
        <FormField
          control={control}
          name="boolEnabled"
          render={({ field }) => (
            <FormItem>
              <FormLabel className="flex items-center gap-1">
                Enabled<Req />
                <Info text="Whether access is granted (on) or explicitly revoked (off)." />
              </FormLabel>
              <FormControl>
                <RadioGroup
                  value={field.value ? 'true' : 'false'}
                  onValueChange={v => field.onChange(v === 'true')}
                  className="flex gap-4"
                >
                  <div className="flex items-center gap-1.5">
                    <RadioGroupItem value="true" id={`${idPrefix}-bool-on`} />
                    <Label htmlFor={`${idPrefix}-bool-on`} className="font-normal cursor-pointer">
                      Enabled
                    </Label>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <RadioGroupItem value="false" id={`${idPrefix}-bool-off`} />
                    <Label htmlFor={`${idPrefix}-bool-off`} className="font-normal cursor-pointer">
                      Disabled
                    </Label>
                  </div>
                </RadioGroup>
              </FormControl>
            </FormItem>
          )}
        />
      </Fragment>
    )
  }

  return (
    <Fragment key="metered">
      <FormField
        control={control}
        name="limit"
        render={({ field }) => (
          <FormItem>
            <FormLabel className="flex items-center gap-1">
              Limit<Opt />
              <Info text="Maximum usage allowed per reset period. Leave blank for unlimited." />
            </FormLabel>
            <FormControl>
              <Input type="number" min={0} placeholder="∞ Unlimited" {...field} />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
      <FormField
        control={control}
        name="resetPeriodType"
        render={({ field }) => (
          <FormItem>
            <FormLabel className="flex items-center gap-1">
              Reset period<Req />
            </FormLabel>
            <Select value={field.value} onValueChange={field.onChange}>
              <FormControl>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="never">Never (lifetime cap)</SelectItem>
                <SelectItem value="billingCycle">Billing cycle</SelectItem>
                <SelectItem value="calendar">Calendar</SelectItem>
                <SelectItem value="fixedWindow">Fixed window</SelectItem>
                <SelectItem value="slidingWindow">Sliding window</SelectItem>
              </SelectContent>
            </Select>
            {field.value && (
              <p className="text-xs text-muted-foreground mt-1">
                {resetPeriodHint(field.value)}
              </p>
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
                  Every<Req />
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
                  Unit<Req />
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
      <FormField
        control={control}
        name="overageBehaviorType"
        render={({ field }) => (
          <FormItem>
            <FormLabel className="flex items-center gap-1">
              Overage behavior<Req />
            </FormLabel>
            <Select value={field.value} onValueChange={field.onChange}>
              <FormControl>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="block">Block at cap</SelectItem>
                <SelectItem value="allow">Allow overage</SelectItem>
              </SelectContent>
            </Select>
            {field.value && (
              <p className="text-xs text-muted-foreground mt-1">
                {overageBehaviorHint(field.value)}
              </p>
            )}
          </FormItem>
        )}
      />
      {showGracePeriod && (
        <FormField
          control={control}
          name="gracePeriodPct"
          render={({ field }) => (
            <FormItem>
              <FormLabel className="flex items-center gap-1">
                Grace period <span className="text-muted-foreground text-xs">(%)</span>
                <Opt />
                <Info text="Extra usage allowed beyond the limit before hard enforcement, as a percentage of the limit. Only applies when Block is selected." />
              </FormLabel>
              <FormControl>
                <Input
                  type="number"
                  min={0}
                  placeholder="—"
                  {...field}
                  value={field.value ?? ''}
                  onChange={e => field.onChange(e.target.value === '' ? undefined : Number(e.target.value))}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      )}
      <FormField
        control={control}
        name="warningThresholdPct"
        render={({ field }) => (
          <FormItem>
            <FormLabel className="flex items-center gap-1">
              Warning threshold <span className="text-muted-foreground text-xs">(%)</span>
              <Opt />
              <Info text="Percentage of the limit at which a warning signal is emitted. Your integration can use this to notify users before they hit the cap." />
            </FormLabel>
            <FormControl>
              <Input
                type="number"
                min={0}
                max={100}
                placeholder="—"
                {...field}
                value={field.value ?? ''}
                onChange={e => field.onChange(e.target.value === '' ? undefined : Number(e.target.value))}
              />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
    </Fragment>
  )
}
