import { DatePicker, GenericFormField, InputFormField } from '@md/ui'
import { addDays, startOfDay } from 'date-fns'
import { useFormContext } from 'react-hook-form'

import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { SubscribablePlanVersionSelect } from '@/features/plans/SubscribablePlanVersionSelect'
import { EditorSection } from '@/features/quotes/editor/EditorSection'
import { useBasePath } from '@/hooks/useBasePath'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'

/** The usual windows for a quote to stay open: from one to twelve weeks, from today. */
const EXPIRY_DAYS = [7, 14, 30, 60, 90]

const expiryPresets = () => {
  const today = startOfDay(new Date())
  return EXPIRY_DAYS.map(days => ({ label: `In ${days} days`, date: addDays(today, days) }))
}

interface DetailsSectionProps {
  /** The plan's currency, which prices the quote: shown under the plan once chosen. */
  currency?: string
  /** Customer, number and expiry; a template has none, only its plan. */
  dealFields?: boolean
  /** A new quote without its customer yet: only that field, the rest follows once chosen. */
  customerOnly?: boolean
}

/** What the quote is: for whom, under which number until when, and on which plan. */
export const DetailsSection = ({
  currency,
  dealFields = true,
  customerOnly = false,
}: DetailsSectionProps) => {
  const { control } = useFormContext<QuoteFormValues>()
  const basePath = useBasePath()

  return (
    <EditorSection id="quote-details" title={dealFields ? 'Quote details' : 'Plan'}>
      <div className="grid grid-cols-1 gap-4 @md:grid-cols-2">
        {dealFields && (
          <GenericFormField
            control={control}
            layout="vertical"
            label="Customer"
            name="customer_id"
            containerClassName="@md:col-span-2"
            render={({ field }) => (
              <CustomerSelect
                value={field.value}
                onChange={field.onChange}
                variant="card"
                placeholder="Select a customer…"
              />
            )}
          />
        )}

        {!customerOnly && (
          <>
            {dealFields && (
              <>
                <InputFormField
                  name="quote_number"
                  label="Quote number"
                  control={control}
                  placeholder="Q-12345"
                  layout="vertical"
                />
                <GenericFormField
                  control={control}
                  layout="vertical"
                  label="Expires on"
                  name="expires_at"
                  render={({ field }) => (
                    <DatePicker
                      mode="single"
                      captionLayout="dropdown"
                      placeholder="No expiry"
                      date={field.value}
                      onSelect={field.onChange}
                      presets={expiryPresets()}
                      clearable
                    />
                  )}
                />
              </>
            )}
            <GenericFormField
              control={control}
              name="plan_version_id"
              label="Plan"
              layout="vertical"
              containerClassName="@md:col-span-2"
              description={currency ? `Priced in ${currency}` : undefined}
              render={({ field }) => (
                <SubscribablePlanVersionSelect
                  value={field.value}
                  onChange={field.onChange}
                  triggerClassName="w-full"
                  placeholder="Select a plan…"
                  createPlanHref={`${basePath}/plans/add-plan`}
                />
              )}
            />
          </>
        )}
      </div>
    </EditorSection>
  )
}
