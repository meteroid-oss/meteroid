import { Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { keepPreviousData } from '@tanstack/react-query'
import { HelpCircleIcon } from 'lucide-react'

import { Switcher } from '@/features/plans/ladder/Switcher'
import { optionSavingLabel } from '@/features/plans/pricingpage/cardPrice'
import { useQuery } from '@/lib/connectrpc'
import { getPricingPage } from '@/rpc/api/pricingpage/v1/pricingpage-PricingPageService_connectquery'
import { PageOption } from '@/rpc/api/pricingpage/v1/pricingpage_pb'

/** The options a product line's plans are sold in, each with its saving: "Yearly · 2 months
 * free". */
export const OptionSwitch = ({
  familyId,
  currency,
  options,
  value,
  onChange,
  size,
}: {
  familyId: string
  currency: string
  options: PageOption[]
  value: string | undefined
  onChange: (alias: string) => void
  size?: 'sm' | 'md'
}) => (
  <Switcher
    label="Option"
    size={size}
    value={value}
    onChange={onChange}
    items={options.map(option => ({
      value: option.alias,
      label: <OptionLabel familyId={familyId} currency={currency} option={option} />,
    }))}
  />
)

// Each option's page is read for its cards' saving; switching to it then shows at once.
const OptionLabel = ({
  familyId,
  currency,
  option,
}: {
  familyId: string
  currency: string
  option: PageOption
}) => {
  const page = useQuery(
    getPricingPage,
    { productFamilyId: familyId, currency, option: option.alias },
    { placeholderData: keepPreviousData }
  ).data?.page
  const saving = page && page.option === option.alias ? optionSavingLabel(page.cards) : undefined
  return (
    <>
      {option.name}
      {saving && (
        <>
          <span className="text-success"> · {saving}</span>
          <SavingHelp />
        </>
      )}
    </>
  )
}

const SAVING_HELP =
  "Compares each plan's fixed fees with 12 months of its monthly price; seats and usage aren't included."

// Inside the switch's button: a span, never a nested button.
const SavingHelp = () => (
  <Tooltip>
    <TooltipTrigger asChild>
      <span className="ml-1 inline-flex cursor-help align-middle text-muted-foreground">
        <HelpCircleIcon size={11} aria-label={SAVING_HELP} />
      </span>
    </TooltipTrigger>
    <TooltipContent className="max-w-56">{SAVING_HELP}</TooltipContent>
  </Tooltip>
)
