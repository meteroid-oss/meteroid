import {
  GenericFormField,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useWatch, type Control, type FieldValues } from 'react-hook-form'

import { useQuery } from '@/lib/connectrpc'
import { listCreditUnits } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'

import { useCreditUnit } from './CreditUnitPriceFields'
import { isZeroAmount } from './schemas'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'

// Radix selects cannot carry an empty value; the form keeps '' for the plan currency.
const PLAN_CURRENCY = '__plan_currency__'

/** The credit unit a credit pack form grants, if any. */
export function useGrantedCreditUnit(control: Control<FieldValues>): CreditUnit | undefined {
  const unitId = useWatch({ control, name: 'grantCreditUnitId' }) as string | undefined
  return useCreditUnit(unitId)
}

/** "Credit granted in" selector of a credit pack: the plan currency or a credit unit. */
export const CreditPackUnitField = ({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) => {
  const unitsQuery = useQuery(listCreditUnits, { includeArchived: true })
  const unitId = useWatch({ control, name: 'grantCreditUnitId' }) as string | undefined
  const included = isZeroAmount(useWatch({ control, name: 'unitPrice' }))
  const units = unitsQuery.data?.units ?? []
  // An archived unit stays listed only on the pack that already grants it.
  const options = units.filter(unit => !unit.archived || unit.id === unitId)

  if (options.length === 0) return null

  return (
    <GenericFormField
      control={control}
      name="grantCreditUnitId"
      label="Credit granted in"
      description={
        !unitId
          ? undefined
          : included
            ? 'Included in the plan: not charged or taxed.'
            : 'Paid in the plan currency, taxed when sold.'
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
  )
}
