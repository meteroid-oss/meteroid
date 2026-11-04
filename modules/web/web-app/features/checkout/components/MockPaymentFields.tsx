import { Building, CreditCard, FlaskConical } from 'lucide-react'

import { MockRail, scenariosForRail } from '@/features/checkout/mockScenarios'

interface MockPaymentFieldsProps {
  rail: MockRail
  scenarioId: string
  onScenarioChange: (id: string) => void
}

/** Scenario picker for the test gateway. Deliberately has NO input of any kind: a card-number
 *  field here would be a card-number field in production code. */
export const MockPaymentFields: React.FC<MockPaymentFieldsProps> = ({
  rail,
  scenarioId,
  onScenarioChange,
}) => {
  const scenarios = scenariosForRail(rail)

  return (
    <div className="mt-4 mb-6 space-y-3">
      <div
        className="flex items-start gap-2 rounded-md px-3 py-2 text-xs"
        style={{ background: 'var(--mtp-surface-2)', color: 'var(--mtp-text-2)' }}
      >
        <FlaskConical size={14} className="mt-0.5 shrink-0" />
        <span>
          Test gateway — pick a scenario. No card or bank details are asked for, collected, or sent
          anywhere.
        </span>
      </div>

      <fieldset className="space-y-2">
        <legend className="sr-only">Test scenario</legend>
        {scenarios.map(scenario => {
          const selected = scenario.id === scenarioId
          const Icon = scenario.rail === 'card' ? CreditCard : Building
          return (
            <label
              key={scenario.id}
              className="w-full flex items-center p-3 border rounded-md text-left cursor-pointer"
              style={{
                borderColor: selected ? 'var(--mtp-accent)' : 'var(--mtp-border)',
                background: selected ? 'var(--mtp-accent-weak)' : 'transparent',
              }}
            >
              <input
                type="radio"
                name="mock-scenario"
                value={scenario.id}
                checked={selected}
                onChange={() => onScenarioChange(scenario.id)}
                className="sr-only"
              />
              <div
                className="w-3 h-3 rounded-full border flex items-center justify-center mr-3 shrink-0"
                style={{ borderColor: selected ? 'var(--mtp-accent)' : 'var(--mtp-border-2)' }}
              >
                {selected && (
                  <div
                    className="w-2 h-2 rounded-full"
                    style={{ background: 'var(--mtp-accent)' }}
                  />
                )}
              </div>
              <Icon size={18} className="mr-3 shrink-0" style={{ color: 'var(--mtp-text-2)' }} />
              <div className="min-w-0">
                <div className="text-sm font-medium truncate">{scenario.display}</div>
                <div className="text-xs" style={{ color: 'var(--mtp-text-2)' }}>
                  {scenario.outcome}
                </div>
              </div>
            </label>
          )
        })}
      </fieldset>
    </div>
  )
}
