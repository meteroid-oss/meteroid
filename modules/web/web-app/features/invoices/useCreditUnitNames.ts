import { useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { listCreditUnits } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'

import type { CreditUnitNames } from '@/features/subscriptions/subscriptionPrice'

/** The tenant's credit unit names by id, archived ones included: documents name past sales. */
export const useCreditUnitNames = (enabled: boolean): Record<string, CreditUnitNames> => {
  const unitsQuery = useQuery(listCreditUnits, { includeArchived: true }, { enabled })
  return useMemo(
    () =>
      Object.fromEntries(
        (unitsQuery.data?.units ?? []).map(unit => [
          unit.id,
          { name: unit.name, pluralName: unit.pluralName, precision: unit.precision },
        ])
      ),
    [unitsQuery.data]
  )
}
