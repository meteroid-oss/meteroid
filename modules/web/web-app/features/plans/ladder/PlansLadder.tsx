import { skipToken } from '@connectrpc/connect-query'
import {
  type Announcements,
  DndContext,
  DragEndEvent,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
} from '@dnd-kit/core'
import { restrictToHorizontalAxis, restrictToParentElement } from '@dnd-kit/modifiers'
import {
  SortableContext,
  rectSortingStrategy,
  sortableKeyboardCoordinates,
} from '@dnd-kit/sortable'
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
import { ColumnDef, PaginationState } from '@tanstack/react-table'
import { CSSProperties, useMemo, useState } from 'react'

import { StandardTable } from '@/components/table/StandardTable'
import { planKind } from '@/features/plans/PlansTable'
import { LadderCard, compactMoney } from '@/features/plans/ladder/LadderCard'
import { canBeRanked, ladderOf, moveWithin } from '@/features/plans/ladder/ladderOrder'
import { useToggleSalesLed } from '@/features/plans/ladder/useSalesLed'
import { useSaveLadder } from '@/features/plans/ladder/useSaveLadder'
import { OptionSwitch } from '@/features/plans/pricingpage/OptionSwitch'
import { PlanPricingPageSheet } from '@/features/plans/pricingpage/PlanPricingPageSheet'
import { useCurrency } from '@/hooks/useCurrency'
import { useQuery } from '@/lib/connectrpc'
import { PlanOverview, PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { getPricingPage } from '@/rpc/api/pricingpage/v1/pricingpage-PricingPageService_connectquery'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

export interface ProductLine {
  id: string
  name: string
}

export interface PageSelection {
  /** Unset: the page's default. */
  currency?: string
  option?: string
  onCurrency: (currency: string) => void
  onOption: (alias: string) => void
}

interface Props extends PageSelection {
  familyId: string | undefined
  lines: ProductLine[]
  onLine: (id: string) => void
  /** Every plan of the product line, unfiltered: a ladder is saved whole. */
  plans: PlanOverview[]
  isLoading: boolean
  canEdit: boolean
}

// Drags move a card along its own row only.
const MODIFIERS = [restrictToHorizontalAxis, restrictToParentElement]

/** A product line's self-serve ladder, left to right, then its plans off it. */
export const PlansLadder = ({
  familyId,
  lines,
  onLine,
  plans,
  isLoading,
  canEdit,
  currency,
  option,
  onCurrency,
  onOption,
}: Props) => {
  const { pending, save } = useSaveLadder()
  const toggleSalesLed = useToggleSalesLed()
  const reportingCurrency = useCurrency().currency
  const [selected, setSelected] = useState<string>()
  const [editing, setEditing] = useState<PlanOverview>()

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates })
  )

  const page = useQuery(
    getPricingPage,
    familyId ? { productFamilyId: familyId, currency, option } : skipToken,
    { placeholderData: keepPreviousData }
  ).data?.page
  const tenantCurrencies = useQuery(listTenantCurrencies).data?.currencies ?? []

  const ladder = ladderOf(plans, familyId ? pending[familyId] : undefined)
  const ids = ladder.map(plan => plan.id)
  const off = plans.filter(plan => !ids.includes(plan.id))

  const priced = plans.flatMap(plan => plan.pricedOptions.map(p => p.currency))
  // The page's own currency too: a ladder of plans with no price (free) still shows in one.
  const currencies = [
    ...new Set([...(page?.currencies ?? []), ...priced, ...(page?.currency ? [page.currency] : [])]),
  ].sort()
  const shownCurrency = page?.currency || currency || currencies[0]
  const shownOption = page?.options.find(o => o.alias === page.option)

  const onSave = canEdit && familyId ? (order: string[]) => save(familyId, order) : undefined
  const move = (id: string, target: string) => {
    const next = moveWithin(ids, ids, id, target)
    if (next !== ids) onSave?.(next)
  }
  const onDragEnd = ({ active, over }: DragEndEvent) => {
    if (over && active.id !== over.id) move(String(active.id), String(over.id))
  }
  const nameOf = (id: string | number) => ladder.find(p => p.id === id)?.name ?? 'the plan'
  const placeOf = (id: string | number) => ids.indexOf(String(id)) + 1
  const announcements: Announcements = {
    onDragStart: ({ active }) => `Picked up ${nameOf(active.id)}, rank ${placeOf(active.id)}.`,
    onDragOver: ({ active, over }) =>
      over
        ? `${nameOf(active.id)} is over rank ${placeOf(over.id)}, ${nameOf(over.id)}.`
        : `${nameOf(active.id)} is no longer over the ladder.`,
    onDragEnd: ({ active, over }) =>
      over
        ? `${nameOf(active.id)} moved to rank ${placeOf(over.id)}.`
        : `${nameOf(active.id)} was dropped.`,
    onDragCancel: ({ active }) =>
      `Moving ${nameOf(active.id)} was cancelled. It stays rank ${placeOf(active.id)}.`,
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg border border-border bg-card px-3 py-2.5">
        <LineSelect lines={lines} value={familyId} onChange={onLine} />
        {currencies.length > 0 && (
          <>
            {lines.length > 1 && <span aria-hidden className="h-5 w-px bg-border" />}
            <label className="flex items-center gap-2 text-sm text-muted-foreground">
              Prices from
              <Select value={shownCurrency} onValueChange={onCurrency}>
                <SelectTrigger className="h-8 w-28 font-mono text-xs text-foreground">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {currencies.map(code => (
                    <SelectItem key={code} value={code} className="font-mono text-xs">
                      {code}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          </>
        )}
        {familyId && page && page.options.length > 1 && (
          <OptionSwitch
            familyId={familyId}
            currency={page.currency}
            options={page.options}
            value={page.option}
            onChange={onOption}
            size="sm"
          />
        )}
        {onSave && ladder.length > 1 && (
          <span className="ml-auto text-xs text-muted-foreground">
            Drag a plan, or use its arrows. Customers upgrade to the right.
          </span>
        )}
      </div>

      {isLoading ? (
        <CardsSkeleton />
      ) : ladder.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border px-4 py-10 text-center text-sm text-muted-foreground">
          {onSave && off.some(canJoinLadder)
            ? 'No plan is on the ladder yet: add one from below.'
            : 'No plan is on the ladder.'}
        </div>
      ) : (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          modifiers={MODIFIERS}
          onDragEnd={onDragEnd}
          accessibility={{ announcements }}
        >
          <SortableContext items={ids} strategy={rectSortingStrategy}>
            {/* Two columns on narrow screens, one row in ladder order from lg up. */}
            <div
              style={{ '--cards': ladder.length } as CSSProperties}
              className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:[grid-template-columns:repeat(var(--cards),minmax(0,1fr))]"
            >
              {ladder.map((plan, index) => (
                <LadderCard
                  key={plan.id}
                  plan={plan}
                  position={index + 1}
                  card={page?.cards.find(card => card.planId === plan.id)}
                  currency={shownCurrency}
                  option={shownOption}
                  tenantCurrencies={tenantCurrencies}
                  reportingCurrency={reportingCurrency}
                  selected={selected === plan.id}
                  onSelect={() => setSelected(plan.id)}
                  first={index === 0}
                  last={index === ladder.length - 1}
                  onMove={
                    onSave
                      ? step => {
                          const target = ids[index + step]
                          if (target) move(plan.id, target)
                        }
                      : undefined
                  }
                  onEdit={canEdit ? () => setEditing(plan) : undefined}
                  onToggleSalesLed={canEdit ? () => void toggleSalesLed(plan) : undefined}
                  onRemove={onSave ? () => onSave(ids.filter(id => id !== plan.id)) : undefined}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>
      )}

      {!isLoading && off.length > 0 && (
        <OffLadder
          plans={off}
          reportingCurrency={reportingCurrency}
          onAdd={onSave ? plan => onSave([...ids, plan.id]) : undefined}
        />
      )}

      {editing && (
        <PlanPricingPageSheet
          planId={editing.id}
          planName={editing.name}
          planVersionId={editing.activeVersion?.id}
          onClose={() => setEditing(undefined)}
        />
      )}
    </div>
  )
}

/** Whether a plan off the ladder may be put on it. */
const canJoinLadder = (plan: PlanOverview) =>
  canBeRanked(plan) && plan.planStatus !== PlanStatus.INACTIVE

const whyOff = (plan: PlanOverview) => {
  if (plan.planStatus === PlanStatus.ARCHIVED) return 'Archived'
  if (plan.planType === PlanType.CUSTOM) return 'Custom plans are sold by quote'
  if (plan.planStatus === PlanStatus.INACTIVE) return 'Inactive'
  return 'Not on the pricing page'
}

const OFF_PAGE_SIZE = 20

// Every cell centred on the row's height, like the plans table.
const box = 'flex min-h-10 flex-col justify-center text-sm'

/** The product line's plans not on the ladder, a row opening each; those that can be, added. */
const OffLadder = ({
  plans,
  reportingCurrency,
  onAdd,
}: {
  plans: PlanOverview[]
  reportingCurrency?: string
  onAdd?: (plan: PlanOverview) => void
}) => {
  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: OFF_PAGE_SIZE,
  })
  const columns = useMemo<ColumnDef<PlanOverview>[]>(
    () => [
      {
        id: 'name',
        header: 'Plan',
        cell: ({ row }) => {
          const plan = row.original
          const kind = planKind(plan)
          return (
            <div className={box}>
              <div className="truncate font-medium">{plan.name}</div>
              {(kind || plan.description) && (
                <div className="max-w-xs truncate text-xs text-muted-foreground">
                  {[kind, plan.description].filter(Boolean).join(' · ')}
                </div>
              )}
            </div>
          )
        },
      },
      {
        id: 'why',
        header: 'Why it\'s off',
        cell: ({ row }) => (
          <div className={cn(box, 'text-muted-foreground')}>{whyOff(row.original)}</div>
        ),
      },
      {
        id: 'subscriptions',
        header: () => <div className="text-right">Subscriptions</div>,
        cell: ({ row }) => (
          <div className={cn(box, 'items-end tabular-nums')}>{row.original.subscriptionCount}</div>
        ),
      },
      {
        id: 'mrr',
        header: () => <div className="text-right">MRR</div>,
        cell: ({ row }) => (
          <div className={cn(box, 'items-end tabular-nums text-muted-foreground')}>
            {row.original.mrr !== undefined && reportingCurrency
              ? compactMoney(row.original.mrr, reportingCurrency)
              : '—'}
          </div>
        ),
      },
      {
        id: 'actions',
        header: '',
        meta: { skipLink: true },
        cell: ({ row }) =>
          onAdd && canJoinLadder(row.original) ? (
            <div className={cn(box, 'items-end')}>
              <Button
                variant="outline"
                size="sm"
                className="mt-0"
                onClick={event => {
                  event.stopPropagation()
                  onAdd(row.original)
                }}
              >
                Add to ladder
              </Button>
            </div>
          ) : null,
      },
    ],
    [onAdd, reportingCurrency]
  )
  const start = pagination.pageIndex * pagination.pageSize

  return (
    <section aria-labelledby="off-ladder" className="space-y-3">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <h2 id="off-ladder" className="text-lg font-semibold">
          Off the ladder
        </h2>
        <span className="text-sm text-muted-foreground">
          Not offered in the portal or on the pricing page
        </span>
      </div>
      <StandardTable
        columns={columns}
        data={plans.slice(start, start + pagination.pageSize)}
        pagination={pagination}
        setPagination={setPagination}
        totalCount={plans.length}
        rowLink={row => row.original.localId}
      />
    </section>
  )
}

/** "Product line [Analytics]". */
export const LineSelect = ({
  lines,
  value,
  onChange,
  className,
}: {
  lines: ProductLine[]
  value: string | undefined
  onChange: (id: string) => void
  className?: string
}) =>
  // Most workspaces have one product line: there is nothing to pick, so nothing shows.
  lines.length < 2 ? null : (
  <label className={cn('flex items-center gap-2 text-sm text-muted-foreground', className)}>
    Product line
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger className="h-8 w-40 text-foreground">
        <SelectValue placeholder="None yet" />
      </SelectTrigger>
      <SelectContent>
        {lines.map(line => (
          <SelectItem key={line.id} value={line.id}>
            {line.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  </label>
  )

const CardsSkeleton = () => (
  <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
    {[0, 1, 2, 3].map(i => (
      <Skeleton key={i} className="h-80 w-full rounded-xl" />
    ))}
  </div>
)
