import { useState } from 'react'

import {
  PriceComponentsLogic,
  PriceComponentsState,
} from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'

interface QuotePriceComponentsWrapperProps {
  planVersionId: PlanVersion['id']
  currency: string
  onValidationChange?: (isValid: boolean, errors: string[]) => void
  onStateChange?: (state: PriceComponentsState) => void
  initialState?: PriceComponentsState
  exampleAmountByKey?: Map<string, string | undefined>
}

export const QuotePriceComponentsWrapper = ({
  planVersionId,
  currency,
  onValidationChange,
  onStateChange,
  initialState,
  exampleAmountByKey,
}: QuotePriceComponentsWrapperProps) => {
  const [state, setState] = useState<PriceComponentsState>(
    initialState || {
      components: {
        removed: [],
        parameterized: [],
        overridden: [],
        extra: [],
        usageExamples: [],
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
      currency={currency}
      state={state}
      onStateChange={handleStateChange}
      onValidationChange={onValidationChange}
      showUsageExamples
      exampleAmountByKey={exampleAmountByKey}
    />
  )
}
