import { useState } from 'react'

import {
  MockRail,
  defaultScenarioId,
  findScenario,
  mintMockPaymentMethodId,
} from '@/features/checkout/mockScenarios'

/** Scenario selection for the test gateway; the confirm step mints the method id from it. */
export const useMockScenario = (rail: MockRail) => {
  const [scenarioId, setScenarioId] = useState(() => defaultScenarioId(rail))

  const confirmNewMethod = async () => {
    const scenario = findScenario(scenarioId)
    if (!scenario) {
      throw new Error('Pick a test scenario')
    }
    return mintMockPaymentMethodId(scenario)
  }

  return { scenarioId, setScenarioId, confirmNewMethod }
}
