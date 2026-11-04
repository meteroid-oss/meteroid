import {
  GenericFormField,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { createContext, useContext, useEffect } from 'react'
import { useController, useWatch, type Control, type FieldValues } from 'react-hook-form'
import { Link } from 'react-router-dom'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { creditValue } from '@/features/pricing/creditValue'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { listCreditUnits } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'

// Radix selects cannot carry an empty value; the form keeps '' for the plan currency.
const PLAN_CURRENCY = '__plan_currency__'

/** Set where access is denied once the credit runs out: an empty overage price bills nothing. */
export const CreditRateOptionalContext = createContext(false)

/** The tenant's credit units, archived ones included. */
export function useCreditUnits(): CreditUnit[] | undefined {
  return useQuery(listCreditUnits, { includeArchived: true }).data?.units
}

/** The tenant's credit unit `creditUnitId` names, archived ones included. */
export function useCreditUnit(creditUnitId: string | undefined): CreditUnit | undefined {
  const units = useCreditUnits()
  return creditUnitId ? units?.find(unit => unit.id === creditUnitId) : undefined
}

/** The credit unit a usage or capacity price form is expressed in, if any. */
export function useSelectedCreditUnit(control: Control<FieldValues>): CreditUnit | undefined {
  const creditUnitId = useWatch({ control, name: 'creditUnitId' }) as string | undefined
  return useCreditUnit(creditUnitId)
}

/**
 * "Priced in" selector for usage and capacity prices: the plan currency or a credit unit, with
 * the rate at which credits the customer's balance does not cover are billed in money.
 */
export const CreditUnitPriceFields = ({
  control,
  currency,
  overageOnly,
}: {
  control: Control<FieldValues>
  currency: string
  overageOnly?: boolean
}) => {
  const basePath = useBasePath()
  const rateOptional = useContext(CreditRateOptionalContext)
  const { field: rateOptionalField } = useController({ control, name: 'creditRateOptional' })
  const setRateOptional = rateOptionalField.onChange
  useEffect(() => {
    setRateOptional(rateOptional)
  }, [rateOptional, setRateOptional])
  const unitsQuery = useQuery(listCreditUnits, { includeArchived: true })
  const creditUnitId = useWatch({ control, name: 'creditUnitId' }) as string | undefined
  const units = unitsQuery.data?.units ?? []
  const selected = units.find(unit => unit.id === creditUnitId)
  const value = creditValue(selected, currency)
  const { field: valuedField } = useController({ control, name: 'creditUnitValued' })
  const setValued = valuedField.onChange
  useEffect(() => {
    setValued(!!value)
  }, [value, setValued])
  // An archived unit stays listed only on the price that already uses it.
  const options = units.filter(unit => !unit.archived || unit.id === creditUnitId)

  if (options.length === 0) return null

  return (
    <div className="grid grid-cols-2 gap-3 items-start">
      <GenericFormField
        control={control}
        name="creditUnitId"
        label={overageOnly ? 'Overage priced in' : 'Priced in'}
        description={
          selected ? `Prices in ${selected.pluralName}, paid from the balance first` : undefined
        }
        render={({ field }) => (
          <Select
            name={field.name}
            value={field.value || PLAN_CURRENCY}
            onValueChange={value => field.onChange(value === PLAN_CURRENCY ? '' : value)}
          >
            <SelectTrigger ref={field.ref} className="max-w-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={PLAN_CURRENCY}>{currency}</SelectItem>
              {options.map(unit => (
                <SelectItem key={unit.id} value={unit.id}>
                  {unit.pluralName}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      />
      {selected && (
        <div className="space-y-1">
          <GenericFormField
            control={control}
            name="creditUnitRate"
            label={
              rateOptional || value
                ? 'Overage price per credit (optional)'
                : 'Overage price per credit'
            }
            description={
              rateOptional
                ? 'Access is denied at zero under this plan: left empty, usage that still arrives past zero is not billed.'
                : value
                  ? 'Billed for credits the balance does not cover.'
                  : undefined
            }
            render={({ field }) => (
              <UncontrolledPriceInput
                {...field}
                currency={currency}
                precision={8}
                placeholder={
                  rateOptional
                    ? 'Not billed'
                    : value
                      ? `Uses the credit's value (${formatCurrencyNoRounding(value, currency)})`
                      : '0.00'
                }
                className="max-w-xs"
              />
            )}
          />
          {!rateOptional && !value && (
            <p className="text-xs text-muted-foreground">
              {selected.pluralName} have no money value in {currency}.{' '}
              <Link to={`${basePath}/settings/credit-units`} className="text-brand hover:underline">
                Set their value
              </Link>{' '}
              or enter an overage price here.
            </p>
          )}
        </div>
      )}
    </div>
  )
}
