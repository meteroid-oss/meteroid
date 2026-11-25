import { useAtom } from 'jotai'

import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'
import { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

import { usePlanVersionFamily } from './ChargeDialogs'
import { PriceComponentsLogic } from './PriceComponentsLogic'

interface CreateSubscriptionPriceComponentsProps {
  /** The plan's charges as bought in the chosen option. */
  planComponents: PriceComponent[]
  currency: string
  /** The plan version picked: its family is where a new fee's products come from. */
  planVersionId?: string
  onValidationChange?: (isValid: boolean, errors: string[]) => void
}

export const CreateSubscriptionPriceComponents = ({
  planComponents,
  currency,
  planVersionId,
  onValidationChange,
}: CreateSubscriptionPriceComponentsProps) => {
  const [state, setState] = useAtom(createSubscriptionAtom)
  const familyLocalId = usePlanVersionFamily(planVersionId)

  return (
    <PriceComponentsLogic
      planComponents={planComponents}
      currency={currency}
      state={state}
      onStateChange={a => setState({ ...state, ...a })}
      onValidationChange={onValidationChange}
      familyLocalId={familyLocalId}
    />
  )
}
