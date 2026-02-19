import { useAtom } from 'jotai'

import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'

import { PriceComponentsLogic } from './PriceComponentsLogic'

interface CreateSubscriptionPriceComponentsProps {
  planVersionId: PlanVersion['id']
  currency: string
  onValidationChange?: (isValid: boolean, errors: string[]) => void
}

export const CreateSubscriptionPriceComponents = ({
  planVersionId,
  currency,
  onValidationChange,
}: CreateSubscriptionPriceComponentsProps) => {
  const [state, setState] = useAtom(createSubscriptionAtom)

  return (
    <PriceComponentsLogic
      planVersionId={planVersionId}
      currency={currency}
      state={state}
      onStateChange={a => setState({ ...state, ...a })}
      onValidationChange={onValidationChange}
    />
  )
}
