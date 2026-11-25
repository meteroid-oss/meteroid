import { useMemo } from 'react'

import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { creditPackContext } from '@/features/pricing/formData'
import { useZodForm } from '@/hooks/useZodForm'

import { chargeFormSchema } from './schema'

import type { ChargeShape, ChargeValues } from './compile'
import type { z } from 'zod'

/** The composer's form: its schema follows the shape, so only the chosen fields are checked. */
export function useChargeForm({
  shape,
  defaultValues,
  currency,
  newProduct,
}: {
  shape: ChargeShape
  defaultValues: ChargeValues
  currency: string
  newProduct: boolean
}) {
  const creditUnits = useCreditUnits()
  const schema = useMemo(
    () =>
      chargeFormSchema(shape, {
        newProduct,
        creditPack: creditPackContext(currency, creditUnits),
      }),
    [shape, newProduct, currency, creditUnits]
  )
  return useZodForm({ schema: schema as z.ZodType, defaultValues })
}
