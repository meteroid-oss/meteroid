import { Input, Label, RadioGroup, RadioGroupItem, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { InfoIcon } from 'lucide-react'
import { useEffect, useId } from 'react'

import { type Attribution, AttributionPreview } from '@/features/productCatalog/metrics/AttributionPreview'
import { TypeSelect } from '@/features/resources/TypeSelect'
import { useResourceTypes } from '@/features/resources/hooks'
import { SettingsFieldGrid, SettingsSection } from '@/features/settings/components/SettingsSection'

import type { ReactNode } from 'react'

export const sameAttribution = (a: Attribution, b: Attribution) =>
  a?.typeId === b?.typeId && (a?.property || undefined) === (b?.property || undefined)

type ReadFrom = 'keys' | 'property'

/** "Attributed to": Customer, or a resource type. A change on a live metric needs its preview first. */
export const AttributionSection = ({
  value,
  onChange,
  metricId,
  changed,
  onPreviewed,
}: {
  value: Attribution
  onChange: (a: Attribution) => void
  /** Set on an existing metric: its last 7 days are previewed under `value`. */
  metricId?: string
  changed?: boolean
  onPreviewed?: (ready: boolean) => void
}) => {
  const id = useId()
  const { byId } = useResourceTypes()
  const type = value ? byId.get(value.typeId) : undefined
  useEffect(() => {
    if (!changed) onPreviewed?.(true)
  }, [changed]) // eslint-disable-line react-hooks/exhaustive-deps
  const readFrom: ReadFrom = value?.property === undefined ? 'keys' : 'property'

  return (
    <SettingsSection title="Attributed to">
      <div className="flex flex-col gap-4">
        <SettingsFieldGrid>
          <Field
            label="Billed to"
            tooltip="Events naming a resource with no subscription of its own go to the customer's subscription."
          >
            <TypeSelect
              value={value?.typeId}
              onChange={typeId => onChange(typeId ? { typeId } : undefined)}
              customerLabel="Customer"
            />
          </Field>
          {type && (
            <Field label="Read from">
              <RadioGroup
                value={readFrom}
                onValueChange={v => onChange({ typeId: type.id, property: v === 'keys' ? undefined : '' })}
                className="flex h-9 items-center gap-4"
              >
                <div className="flex items-center gap-2">
                  <RadioGroupItem value="keys" id={`${id}-keys`} />
                  <Label htmlFor={`${id}-keys`} className="flex cursor-pointer items-baseline gap-1.5 font-normal">
                    {type.name} keys
                    <span className="font-mono text-xs text-muted-foreground">{type.keys.join(', ')}</span>
                  </Label>
                </div>
                <div className="flex items-center gap-2">
                  <RadioGroupItem value="property" id={`${id}-property`} />
                  <Label htmlFor={`${id}-property`} className="cursor-pointer font-normal">
                    Event property
                  </Label>
                </div>
              </RadioGroup>
            </Field>
          )}
          {type && readFrom === 'property' && (
            <Field label="Property">
              <Input
                className="font-mono"
                placeholder={type.keys[0]}
                value={value?.property ?? ''}
                onChange={e => onChange({ typeId: type.id, property: e.target.value })}
              />
            </Field>
          )}
        </SettingsFieldGrid>
        {metricId && changed && (
          <AttributionPreview metricId={metricId} value={value} details onReady={onPreviewed} />
        )}
      </div>
    </SettingsSection>
  )
}

const Field = ({ label, tooltip, children }: { label: string; tooltip?: string; children: ReactNode }) => (
  <div className="space-y-1">
    <div className="my-2 flex items-center gap-1">
      <Label className="text-xs font-normal dark:text-muted-foreground">{label}</Label>
      {tooltip && (
        <Tooltip>
          <TooltipTrigger asChild>
            <InfoIcon className="h-3.5 w-3.5 cursor-help text-muted-foreground" />
          </TooltipTrigger>
          <TooltipContent multiline className="max-w-72">{tooltip}</TooltipContent>
        </Tooltip>
      )}
    </div>
    {children}
  </div>
)
