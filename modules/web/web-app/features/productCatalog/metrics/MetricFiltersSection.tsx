import {
  Button,
  FormDescription,
  GenericFormField,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { Trash2Icon } from 'lucide-react'
import { type Control, useFieldArray, useFormContext, useWatch } from 'react-hook-form'

import { PropertyKeyInput } from '@/features/productCatalog/metrics/PropertyKeyFormField'
import {
  PropertyValueInput,
  PropertyValueTags,
} from '@/features/productCatalog/metrics/PropertyValueField'
import { SettingsSection } from '@/features/settings/components/SettingsSection'

const OPERATORS: { value: 'EQUAL' | 'NOT_EQUAL' | 'IN' | 'NOT_IN'; label: string }[] = [
  { value: 'EQUAL', label: 'equals' },
  { value: 'NOT_EQUAL', label: 'not equals' },
  { value: 'IN', label: 'in' },
  { value: 'NOT_IN', label: 'not in' },
]

/** IN / NOT_IN take a list; EQUAL / NOT_EQUAL take exactly one value. */
const isMulti = (op?: string) => op === 'IN' || op === 'NOT_IN'

interface MetricFiltersSectionProps {
  // The form type differs between create and edit; both carry a `filters` array at the root,
  // so the section is intentionally form-agnostic.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  control: Control<any>
  // Scopes property/value discovery. Create carries `eventCode` as an editable form field
  // (watched below); edit keeps it immutable and outside the form, so it's passed in here.
  eventCode?: string
}

/**
 * Repeatable pre-aggregation filters on event properties. Each filter restricts which
 * events feed the metric (a WHERE, ANDed together) — distinct from segmentation, which
 * splits pricing. `not equals`/`not in` are presence-required: events missing the property
 * are excluded.
 */
export const MetricFiltersSection = ({
  control,
  eventCode: eventCodeProp,
}: MetricFiltersSectionProps) => {
  // setValue is used to collapse the value list when switching to a single-value operator.
  const { setValue } = useFormContext()
  const { fields, append, remove } = useFieldArray({ control, name: 'filters' })
  const filters = useWatch({ control, name: 'filters' }) as
    | { op?: string; values?: string[]; property?: string }[]
    | undefined
  // The metric's event code scopes property/value discovery (empty => free text only). Create
  // watches it from the form; edit supplies it via prop since it lives outside the form there.
  const watchedEventCode = (useWatch({ control, name: 'eventCode' }) as string | undefined) ?? ''
  const eventCode = eventCodeProp ?? watchedEventCode

  // Grid template shared by the header and each row so their columns line up exactly.
  const cols = 'grid grid-cols-1 gap-3 md:grid-cols-[1fr_130px_1.6fr_auto]'

  return (
    <SettingsSection
      title="Filters"
      description="Optional. Restrict which events feed this metric by matching event properties."
    >
      <div className="space-y-4">
        <FormDescription>
          <p>
            Restrict which events count toward this metric by matching event properties. All
            filters must match (AND). Values are matched as text — for a boolean or number, type
            its exact value (e.g. <code className="rounded bg-muted px-1 py-0.5 text-xs">true</code>{' '}
            or <code className="rounded bg-muted px-1 py-0.5 text-xs">42</code>).
          </p>
        </FormDescription>

        {fields.length > 0 && (
          <div className="space-y-3">
            {/* Column headers once, so per-row controls carry no labels and stay aligned even
                when a validation message appears beneath one of them. */}
            <div className={`${cols} hidden md:grid`}>
              <Label className="text-xs text-muted-foreground">Property</Label>
              <Label className="text-xs text-muted-foreground">Operator</Label>
              <Label className="text-xs text-muted-foreground">Value</Label>
              <span />
            </div>

            {fields.map((field, index) => {
              const op = filters?.[index]?.op
              const multi = isMulti(op)
              return (
                <div key={field.id} className={`${cols} items-start`}>
                  <GenericFormField
                    name={`filters.${index}.property`}
                    control={control}
                    render={({ field: propField }) => (
                      <PropertyKeyInput
                        eventCode={eventCode}
                        value={(propField.value as string) ?? ''}
                        onChange={propField.onChange}
                        placeholder="region"
                      />
                    )}
                  />
                  <GenericFormField
                    name={`filters.${index}.op`}
                    control={control}
                    render={({ field: opField }) => (
                      <Select
                        value={opField.value}
                        onValueChange={next => {
                          opField.onChange(next)
                          // Collapse to a single value when leaving a list operator.
                          if (!isMulti(next)) {
                            const current = (filters?.[index]?.values ?? []).slice(0, 1)
                            setValue(`filters.${index}.values`, current, {
                              shouldValidate: true,
                              shouldDirty: true,
                            })
                          }
                        }}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {OPERATORS.map(o => (
                            <SelectItem key={o.value} value={o.value}>
                              {o.label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    )}
                  />
                  <GenericFormField
                    name={`filters.${index}.values`}
                    control={control}
                    render={({ field: valField }) =>
                      multi ? (
                        <PropertyValueTags
                          eventCode={eventCode}
                          propertyKey={filters?.[index]?.property ?? ''}
                          value={valField.value ?? []}
                          onChange={valField.onChange}
                          placeholder="eu-west, us-east"
                        />
                      ) : (
                        <PropertyValueInput
                          eventCode={eventCode}
                          propertyKey={filters?.[index]?.property ?? ''}
                          value={valField.value?.[0] ?? ''}
                          onChange={v => valField.onChange(v ? [v] : [])}
                          placeholder="eu-west"
                        />
                      )
                    }
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label="Remove filter"
                    className="mt-0.5 justify-self-start"
                    onClick={() => remove(index)}
                  >
                    <Trash2Icon className="h-4 w-4 text-muted-foreground" />
                  </Button>
                </div>
              )
            })}
          </div>
        )}

        <div className="space-y-2">
          {fields.length === 0 && (
            <Label className="text-xs text-muted-foreground">No filters — all events count.</Label>
          )}
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => append({ property: '', op: 'EQUAL', values: [] })}
          >
            Add filter
          </Button>
        </div>
      </div>
    </SettingsSection>
  )
}
