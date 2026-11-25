import {
  Button,
  Label,
  MultiSelect,
  MultiSelectContent,
  MultiSelectEmpty,
  MultiSelectItem,
  MultiSelectList,
  MultiSelectSearch,
  MultiSelectTrigger,
  MultiSelectValue,
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { PlusIcon, Trash2Icon } from 'lucide-react'
import { useId } from 'react'

import { ValuesTagInput } from '@/components/form/ValuesTagInput'
import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { useQuery } from '@/lib/connectrpc'
import { listCustomPropertyDefinitions } from '@/rpc/api/customproperties/v1/customproperties-CustomPropertiesService_connectquery'
import {
  CustomPropertyDefinition,
  CustomPropertyEntityType,
  CustomPropertyType,
} from '@/rpc/api/customproperties/v1/models_pb'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

import { eligibilitySummary, fromDraft, newCondition } from './eligibility'

import type { Condition, EligibilityDraft } from './eligibility'

interface Props {
  value: EligibilityDraft
  onChange: (value: EligibilityDraft) => void
  disabled?: boolean
}

/** Which customers may get a plan or redeem a coupon. Never a price. */
export const EligibilityEditor = ({ value, onChange, disabled }: Props) => {
  const id = useId()
  const { conditions } = value
  // No row left: everyone again, so a restricted rule always shows a row.
  const setConditions = (next: Condition[]) =>
    onChange({ restricted: next.length > 0, conditions: next })

  return (
    <div className="space-y-3">
      <RadioGroup
        value={value.restricted ? 'some' : 'everyone'}
        onValueChange={choice =>
          choice === 'some'
            ? setConditions(conditions.length ? conditions : [newCondition([])])
            : setConditions([])
        }
        className="flex flex-wrap gap-4"
        disabled={disabled}
      >
        {[
          { value: 'everyone', label: 'Everyone' },
          { value: 'some', label: 'Only some customers' },
        ].map(option => (
          <div key={option.value} className="flex items-center gap-2">
            <RadioGroupItem value={option.value} id={`${id}-${option.value}`} />
            <Label htmlFor={`${id}-${option.value}`} className="text-sm font-normal">
              {option.label}
            </Label>
          </div>
        ))}
      </RadioGroup>

      {value.restricted && (
        <div className="space-y-2">
          {conditions.map((condition, index) => (
            <ConditionRow
              key={index}
              value={condition}
              countryTaken={conditions.some((c, i) => i !== index && c.kind === 'country')}
              onChange={next => setConditions(conditions.map((c, i) => (i === index ? next : c)))}
              onRemove={() => setConditions(conditions.filter((_, i) => i !== index))}
              disabled={disabled}
            />
          ))}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            hasIcon
            className="h-8 text-xs"
            onClick={() => setConditions([...conditions, newCondition(conditions)])}
            disabled={disabled}
          >
            <PlusIcon size={12} /> Condition
          </Button>
          <p className="text-xs text-muted-foreground" role="status">
            {eligibilitySummary(fromDraft(value), code => getCountryName(code))}
          </p>
        </div>
      )}
    </div>
  )
}

const COUNTRY = 'country'
const PROPERTY = 'property:'

/** The tenant's customer custom properties: what a property condition may ask. */
const useCustomerProperties = () =>
  useQuery(listCustomPropertyDefinitions, {
    entityType: CustomPropertyEntityType.CUSTOMER,
    includeArchived: false,
  }).data?.definitions ?? []

/** The values a property takes from a fixed set; none: free text. */
const fixedValues = (def: CustomPropertyDefinition | undefined) => {
  if (def?.propertyType === CustomPropertyType.BOOLEAN) {
    return [
      { value: 'true', label: 'Yes' },
      { value: 'false', label: 'No' },
    ]
  }
  return (def?.config?.options ?? []).map(o => ({ value: o.value, label: o.label || o.value }))
}

/** What the customer must have: its country, or one of its custom properties. */
const ConditionRow = ({
  value,
  countryTaken,
  onChange,
  onRemove,
  disabled,
}: {
  value: Condition
  /** Another row asks the country already: one row holds every country. */
  countryTaken: boolean
  onChange: (value: Condition) => void
  onRemove: () => void
  disabled?: boolean
}) => {
  const definitions = useCustomerProperties()
  const key = value.kind === 'property' ? value.key : ''
  const definition = definitions.find(def => def.key === key)
  const selected = value.kind === 'country' ? COUNTRY : key ? PROPERTY + key : undefined
  return (
    <div className="flex flex-wrap items-start gap-2">
      <Select
        value={selected}
        onValueChange={choice =>
          onChange(
            choice === COUNTRY
              ? { kind: 'country', countries: [] }
              : { kind: 'property', key: choice.slice(PROPERTY.length), values: [] }
          )
        }
        disabled={disabled}
      >
        <SelectTrigger aria-label="Condition" className="h-9 w-44">
          <SelectValue placeholder="Customer property" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={COUNTRY} disabled={countryTaken}>
            Country
          </SelectItem>
          {definitions.map(def => (
            <SelectItem key={def.key} value={PROPERTY + def.key}>
              {def.name}
            </SelectItem>
          ))}
          {key && !definition && <SelectItem value={PROPERTY + key}>{key}</SelectItem>}
          {definitions.length === 0 && (
            <SelectItem value="none" disabled>
              No customer property defined yet
            </SelectItem>
          )}
        </SelectContent>
      </Select>
      {value.kind === 'country' ? (
        <div className="min-w-48 flex-1">
          <CountriesPicker
            value={value.countries}
            onChange={countries => onChange({ ...value, countries })}
            disabled={disabled}
          />
        </div>
      ) : key ? (
        <PropertyValues
          definition={definition}
          values={value.values}
          onChange={values => onChange({ ...value, values })}
          disabled={disabled}
        />
      ) : null}
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="size-9"
        aria-label="Remove this condition"
        onClick={onRemove}
        disabled={disabled}
      >
        <Trash2Icon size={14} />
      </Button>
    </div>
  )
}

const CountriesPicker = ({
  value,
  onChange,
  disabled,
}: {
  value: string[]
  onChange: (value: string[]) => void
  disabled?: boolean
}) => {
  const countries = useQuery(getCountries).data?.countries ?? []
  const label = (code: string) => `${getCountryFlagEmoji(code)} ${getCountryName(code)}`
  return (
    <MultiSelect value={value} onValueChange={next => onChange(next)} disabled={disabled}>
      <MultiSelectTrigger aria-label="Countries" className="w-full">
        <MultiSelectValue placeholder="Choose countries" maxDisplay={4} maxItemLength={18} />
      </MultiSelectTrigger>
      <MultiSelectContent>
        <MultiSelectSearch placeholder="Search countries…" />
        <MultiSelectList>
          <MultiSelectEmpty>No country found</MultiSelectEmpty>
          {countries.map(country => (
            <MultiSelectItem
              key={country.code}
              value={country.code}
              label={getCountryName(country.code)}
              keywords={[country.code, getCountryName(country.code)]}
            >
              {label(country.code)}
            </MultiSelectItem>
          ))}
        </MultiSelectList>
      </MultiSelectContent>
    </MultiSelect>
  )
}

/** The values the customer's property must hold one of: its options, else free text. */
const PropertyValues = ({
  definition,
  values,
  onChange,
  disabled,
}: {
  definition: CustomPropertyDefinition | undefined
  values: string[]
  onChange: (values: string[]) => void
  disabled?: boolean
}) => {
  const options = fixedValues(definition)
  if (options.length === 0) {
    return (
      <div className="min-h-9 min-w-40 flex-1 rounded-md border border-input px-2 py-1">
        <ValuesTagInput
          value={values}
          onChange={onChange}
          placeholder="is one of, Enter after each"
        />
      </div>
    )
  }
  return (
    <div className="min-w-40 flex-1">
      <MultiSelect value={values} onValueChange={next => onChange(next)} disabled={disabled}>
        <MultiSelectTrigger aria-label="Values" className="w-full">
          <MultiSelectValue placeholder="is one of" maxDisplay={4} maxItemLength={18} />
        </MultiSelectTrigger>
        <MultiSelectContent>
          <MultiSelectList>
            <MultiSelectEmpty>No option</MultiSelectEmpty>
            {options.map(option => (
              <MultiSelectItem key={option.value} value={option.value} label={option.label}>
                {option.label}
              </MultiSelectItem>
            ))}
          </MultiSelectList>
        </MultiSelectContent>
      </MultiSelect>
    </div>
  )
}
