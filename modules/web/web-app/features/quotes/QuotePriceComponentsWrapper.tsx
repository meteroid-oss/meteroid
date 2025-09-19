import { useState } from 'react'

import {
  PriceComponentsLogic,
  PriceComponentsState,
} from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'

interface QuotePriceComponentsWrapperProps {
  planVersionId: PlanVersion['id']
  customerId?: string
  onValidationChange?: (isValid: boolean, errors: string[]) => void
  onStateChange?: (state: PriceComponentsState) => void
  initialState?: PriceComponentsState
}

export const QuotePriceComponentsWrapper = ({
  planVersionId,
  customerId,
  onValidationChange,
  onStateChange,
  initialState,
}: QuotePriceComponentsWrapperProps) => {
  const [state, setState] = useState<PriceComponentsState>(
    initialState || {
      components: {
        removed: [],
        parameterized: [],
        overridden: [],
        extra: [],
      },
    }
  )

  const handleStateChange = (newState: PriceComponentsState) => {
    setState(newState)
    onStateChange?.(newState)
  }

  return (
    <PriceComponentsLogic
      planVersionId={planVersionId}
      customerId={customerId}
      state={state}
      onStateChange={handleStateChange}
      onValidationChange={onValidationChange}
    />
  )
}
