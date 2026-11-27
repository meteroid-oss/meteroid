import {
  InputFormField,
  SelectFormField,
  SelectItem,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { InfoIcon } from 'lucide-react'
import { useFormContext, useWatch } from 'react-hook-form'

import { PERIOD_UNITS, RESETS_OPTIONS } from '@/features/entitlements/model'

const InfoTooltip = ({ children }: { children: React.ReactNode }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <InfoIcon className="h-3.5 w-3.5 cursor-help text-muted-foreground" />
    </TooltipTrigger>
    <TooltipContent multiline className="max-w-72">{children}</TooltipContent>
  </Tooltip>
)

/** Resets, behavior at the limit and alerts of a limit, under `name` (`policy`, `policyOverride`). */
export const PolicyFields = ({ name, appCounted }: { name: string; appCounted: boolean }) => {
  const { control } = useFormContext()
  const [resets, atLimit] = useWatch({ control, name: [`${name}.resets`, `${name}.atLimit`] })

  if (appCounted)
    return (
      <InputFormField
        name={`${name}.thresholds`}
        label="Alerts at (%)"
        control={control}
        placeholder="80, 100"
        labelTooltip={
          <InfoTooltip>
            Checks deny new items once the count reaches the limit. Nothing resets. Alerts fire at
            these percentages of the limit.
          </InfoTooltip>
        }
      />
    )

  const periodic = resets === 'CALENDAR' || resets === 'FIXED' || resets === 'SLIDING'

  return (
    <div className="grid gap-x-4 gap-y-1 sm:grid-cols-2">
      <SelectFormField
        name={`${name}.resets`}
        label="Resets"
        control={control}
        labelTooltip={<InfoTooltip>When the counted usage goes back to zero.</InfoTooltip>}
      >
        {RESETS_OPTIONS.map(([value, label]) => (
          <SelectItem key={value} value={value}>
            {label}
          </SelectItem>
        ))}
      </SelectFormField>
      {periodic && (
        <div className="grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)] items-start gap-2">
          <InputFormField
            name={`${name}.periodInterval`}
            label="Every"
            control={control}
            inputMode="numeric"
          />
          <SelectFormField name={`${name}.periodUnit`} label="Unit" control={control}>
            {PERIOD_UNITS.map(unit => (
              <SelectItem key={unit} value={unit}>
                {unit.toLowerCase()}s
              </SelectItem>
            ))}
          </SelectFormField>
        </div>
      )}
      <SelectFormField
        name={`${name}.atLimit`}
        label="When the limit is reached"
        control={control}
        labelTooltip={
          <InfoTooltip>
            <p>
              <strong>Deny access:</strong> checks return no access once usage passes the limit
              (plus grace). Your app enforces the check; usage is still recorded and billed if the
              plan prices it.
            </p>
            <p className="mt-2">
              <strong>Allow overage:</strong> checks keep granting access and flag usage over the
              limit as overage. It is billed only if the plan has a usage price for this metric.
            </p>
          </InfoTooltip>
        }
      >
        <SelectItem value="DENY">Deny access</SelectItem>
        <SelectItem value="ALLOW_AND_BILL">Allow overage</SelectItem>
      </SelectFormField>
      {atLimit === 'DENY' && (
        <InputFormField
          name={`${name}.gracePct`}
          label="Grace (%)"
          control={control}
          labelTooltip={
            <InfoTooltip>Extra usage allowed past the limit before checks deny access.</InfoTooltip>
          }
          inputMode="numeric"
          placeholder="0"
        />
      )}
      <InputFormField
        name={`${name}.thresholds`}
        label="Alerts at (%)"
        control={control}
        placeholder="80, 100"
        containerClassName="sm:col-span-2"
        labelTooltip={<InfoTooltip>Usage alerts fire at these percentages of the limit.</InfoTooltip>}
      />
    </div>
  )
}
