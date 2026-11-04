/**
 * Test gateway scenarios (backend `MockConnector`). NO INSTRUMENT DATA IS EVER COLLECTED: the
 * customer picks a fixed scenario and only `mock_pm_<behavior>_<method>_<hint>_<random>`, minted
 * from its constants, leaves the page. If this file ever grows an input, a real PAN fits in it.
 */
export type MockBehavior = 'ok' | 'decline' | 'pending' | 'action' | 'reauth'
export type MockRail = 'card' | 'directDebit'
export type MockMethod = 'card' | 'sepa' | 'ach' | 'bacs'

export interface MockScenario {
  id: string
  rail: MockRail
  method: MockMethod
  behavior: MockBehavior
  /** Fixed label of the fake instrument — a constant, never user input. */
  display: string
  /** What the charge will do, shown next to the scenario. */
  outcome: string
  /** Card brand, cards only. */
  brand?: string
  /** Last 4 of the fake instrument; part of the scenario, not of any input. */
  last4: string
}

export const MOCK_SCENARIOS: MockScenario[] = [
  {
    id: 'card-ok',
    rail: 'card',
    method: 'card',
    behavior: 'ok',
    display: 'Visa •••• 4242',
    outcome: 'Payment succeeds',
    brand: 'visa',
    last4: '4242',
  },
  {
    id: 'card-decline',
    rail: 'card',
    method: 'card',
    behavior: 'decline',
    display: 'Visa •••• 0002',
    outcome: 'Card is declined',
    brand: 'visa',
    last4: '0002',
  },
  {
    id: 'card-3ds',
    rail: 'card',
    method: 'card',
    behavior: 'action',
    display: 'Visa •••• 3220',
    outcome: 'Asks for 3DS authentication; renewals stay processing',
    brand: 'visa',
    last4: '3220',
  },
  {
    id: 'card-3ds-always',
    rail: 'card',
    method: 'card',
    behavior: 'reauth',
    display: 'Visa •••• 3184',
    outcome: 'Asks for 3DS on every charge, renewals included',
    brand: 'visa',
    last4: '3184',
  },
  {
    id: 'card-pending',
    rail: 'card',
    method: 'card',
    behavior: 'pending',
    display: 'Mastercard •••• 0119',
    outcome: 'Stays processing until settled from the invoice page',
    brand: 'mastercard',
    last4: '0119',
  },
  {
    id: 'sepa-ok',
    rail: 'directDebit',
    method: 'sepa',
    behavior: 'ok',
    display: 'SEPA account •••• 0009',
    outcome: 'Mandate set up, payment succeeds',
    last4: '0009',
  },
  {
    id: 'sepa-decline',
    rail: 'directDebit',
    method: 'sepa',
    behavior: 'decline',
    display: 'SEPA account •••• 0002',
    outcome: 'Debit is refused',
    last4: '0002',
  },
  {
    id: 'sepa-pending',
    rail: 'directDebit',
    method: 'sepa',
    behavior: 'pending',
    display: 'SEPA account •••• 0119',
    outcome: 'Stays processing until settled from the invoice page',
    last4: '0119',
  },
  {
    id: 'ach-ok',
    rail: 'directDebit',
    method: 'ach',
    behavior: 'ok',
    display: 'ACH account •••• 0009',
    outcome: 'Mandate set up, payment succeeds',
    last4: '0009',
  },
  {
    id: 'bacs-ok',
    rail: 'directDebit',
    method: 'bacs',
    behavior: 'ok',
    display: 'Bacs account •••• 0009',
    outcome: 'Mandate set up, payment succeeds',
    last4: '0009',
  },
]

export const scenariosForRail = (rail: MockRail): MockScenario[] =>
  MOCK_SCENARIOS.filter(scenario => scenario.rail === rail)

export const defaultScenarioId = (rail: MockRail): string => scenariosForRail(rail)[0].id

export const findScenario = (id: string): MockScenario | undefined =>
  MOCK_SCENARIOS.find(scenario => scenario.id === id)

/** `<hint>` is `<brand>-<last4>` for cards and the account's last 4 for a direct debit. */
export const mintMockPaymentMethodId = (scenario: MockScenario): string => {
  const suffix = Math.random().toString(36).slice(2, 10)
  const hint = scenario.method === 'card' ? `${scenario.brand}-${scenario.last4}` : scenario.last4
  return `mock_pm_${scenario.behavior}_${scenario.method}_${hint}_${suffix}`
}
