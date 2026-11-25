import {
  Button,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  cn,
} from '@md/ui'
import { keepPreviousData } from '@tanstack/react-query'
import { CSSProperties, ReactNode, useState } from 'react'

import { OptionSwitch } from '@/features/plans/pricingpage/OptionSwitch'
import { PlanPricingPageSheet } from '@/features/plans/pricingpage/PlanPricingPageSheet'
import { PricingCardView } from '@/features/plans/pricingpage/PricingCardView'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { PlanOverview } from '@/rpc/api/plans/v1/models_pb'
import { getPricingPage } from '@/rpc/api/pricingpage/v1/pricingpage-PricingPageService_connectquery'

interface Props {
  familyId: string
  /** The product line's plans, to edit a card against its published version. */
  plans: PlanOverview[]
  canEdit: boolean
  /** Unset: the page's default. */
  currency?: string
  option?: string
  onCurrency: (currency: string) => void
  onOption: (alias: string) => void
  /** Beside the page's controls, e.g. the product line picker. */
  controls?: ReactNode
}

interface Editing {
  planId: string
  planName: string
  planVersionId?: string
}

/** A product line's public pricing page as a visitor paying in a currency sees it. Its order is
 * the ladder's. */
export const PricingPagePreview = ({
  familyId,
  plans,
  canEdit,
  currency,
  option,
  onCurrency,
  onOption,
  controls,
}: Props) => {
  const [editing, setEditing] = useState<Editing>()

  const query = useQuery(
    getPricingPage,
    { productFamilyId: familyId, currency, option },
    { placeholderData: keepPreviousData }
  )
  const page = query.data?.page

  const edit = (planId: string, planName: string) => {
    const plan = plans.find(p => p.id === planId)
    setEditing({ planId, planName, planVersionId: plan?.activeVersion?.id })
  }

  return (
    <section className="space-y-5">
      <div className="flex flex-wrap items-center justify-center gap-3">
        {controls}
        {page && page.currencies.length > 0 && (
          <label className="flex items-center gap-2 text-sm text-muted-foreground">
            As seen by
            <Select
              value={page.currency}
              onValueChange={onCurrency}
              disabled={page.currencies.length < 2}
            >
              <SelectTrigger className="h-9 w-48 text-foreground">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {page.currencies.map(code => (
                  <SelectItem key={code} value={code}>
                    A visitor paying in {code}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
        )}
        {page && page.options.length > 1 && (
          <OptionSwitch
            familyId={familyId}
            currency={page.currency}
            options={page.options}
            value={page.option}
            onChange={onOption}
          />
        )}
      </div>

      {query.isLoading ? (
        <CardsSkeleton />
      ) : !page ? (
        <div className="flex flex-col items-center gap-3 rounded-lg border border-dashed border-border px-4 py-10 text-center text-sm">
          <span className="text-muted-foreground">
            {rpcErrorMessage(query.error, 'The pricing page could not be loaded.')}
          </span>
          <Button variant="outline" size="sm" onClick={() => query.refetch()}>
            Retry
          </Button>
        </div>
      ) : page.cards.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border px-4 py-10 text-center text-sm text-muted-foreground">
          No plan of this line is on the pricing page: put one on the ladder.
        </div>
      ) : (
        <div
          style={{ '--cards': page.cards.length } as CSSProperties}
          className={cn(
            'grid grid-cols-1 gap-4 transition-opacity sm:grid-cols-2 lg:[grid-template-columns:repeat(var(--cards),minmax(0,1fr))]',
            query.isPlaceholderData && query.isFetching && 'opacity-60'
          )}
        >
          {page.cards.map(card => (
            <PricingCardView
              key={card.planId}
              card={card}
              currency={page.currency}
              option={page.option}
              footer={
                canEdit && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="mt-0 h-7 px-2.5 text-xs text-muted-foreground"
                    onClick={() => edit(card.planId, card.planName)}
                  >
                    Edit copy
                  </Button>
                )
              }
            />
          ))}
        </div>
      )}
      {page && page.notSoldPlanNames.length > 0 && (
        <p className="text-xs text-muted-foreground">
          Not sold in {page.currency}, so not on this page: {page.notSoldPlanNames.join(', ')}.
        </p>
      )}

      {editing && (
        <PlanPricingPageSheet
          planId={editing.planId}
          planName={editing.planName}
          planVersionId={editing.planVersionId}
          onClose={() => setEditing(undefined)}
        />
      )}
    </section>
  )
}

const CardsSkeleton = () => (
  <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
    {[0, 1, 2, 3].map(i => (
      <Skeleton key={i} className="h-96 w-full rounded-xl" />
    ))}
  </div>
)
