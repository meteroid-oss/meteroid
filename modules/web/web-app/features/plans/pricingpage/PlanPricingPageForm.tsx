import { create } from '@bufbuild/protobuf'
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import {
  DndContext,
  DragEndEvent,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
} from '@dnd-kit/core'
import { restrictToParentElement, restrictToVerticalAxis } from '@dnd-kit/modifiers'
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import {
  Button,
  Checkbox,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Switch,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from '@md/ui'
import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import {
  ArrowDownIcon,
  ArrowUpIcon,
  CheckIcon,
  ChevronDownIcon,
  EyeIcon,
  EyeOffIcon,
  GripVerticalIcon,
  HelpCircleIcon,
  PlusIcon,
  TypeIcon,
  XIcon,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'

import { plainValue } from '@/features/entitlements/rows/grantDisplay'
import { inheritedValue } from '@/features/entitlements/rows/grantDraft'
import { useFeatures } from '@/features/entitlements/rows/useFeatures'
import { isEntitlementDisabled } from '@/features/entitlements/utils'
import { type OrderRow, inOrder, movedOrder, orderRows } from '@/features/plans/pricingpage/cardOrder'
import { priceDisplay } from '@/features/plans/pricingpage/cardPrice'
import {
  type DraftHighlight,
  MAX_HIGHLIGHTS,
  MAX_HIGHLIGHT_LENGTH,
  draftKey,
  fromHighlights,
  moveItem,
  toHighlights,
} from '@/features/plans/pricingpage/highlights'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { rpcErrorMessage } from '@/lib/rpcError'
import { getResolvedEntitlementsForPlanVersion } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { EntitlementValueSchema } from '@/rpc/api/entitlements/v1/models_pb'
import {
  getPlanPricingPage,
  getPricingPage,
  updatePlanPricingPage,
} from '@/rpc/api/pricingpage/v1/pricingpage-PricingPageService_connectquery'
import {
  PlanPricingPage,
  PricingCard,
  PricingCharge,
} from '@/rpc/api/pricingpage/v1/pricingpage_pb'

interface Props {
  planId: string
  /** The version whose features can be picked: the published one, else the draft. */
  planVersionId?: string
  readOnly?: boolean
  onCancel?: () => void
  onSaved?: () => void
  /** Classes of the actions row, e.g. to pin it to a sheet's bottom. */
  actionsClassName?: string
  className?: string
}

/** The plan's public pricing page copy: how it's sold, tagline, popular mark, button and highlights. */
export const PlanPricingPageForm = (props: Props) => {
  const query = useQuery(getPlanPricingPage, { planId: props.planId })

  if (query.isLoading) {
    return (
      <div className={cn('space-y-4', props.className)}>
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    )
  }
  if (query.error) {
    return (
      <div className={cn('flex items-center gap-3 text-sm', props.className)}>
        <span className="text-muted-foreground">
          {rpcErrorMessage(query.error, 'The pricing page could not be loaded')}
        </span>
        <Button variant="outline" size="sm" onClick={() => query.refetch()}>
          Retry
        </Button>
      </div>
    )
  }
  return (
    <Editor
      {...props}
      page={query.data?.page}
      charges={query.data?.charges ?? []}
      familyId={query.data?.productFamilyId}
    />
  )
}

interface Draft {
  tagline: string
  popular: boolean
  ctaLabel: string
  highlights: DraftHighlight[]
  salesLed: boolean
  showPrice: boolean
  contactUrl: string
  /** By product; empty: as computed. */
  productOrder: string[]
  hiddenProductIds: string[]
}

const draftOf = (page?: PlanPricingPage): Draft => ({
  tagline: page?.tagline ?? '',
  popular: page?.popular ?? false,
  ctaLabel: page?.ctaLabel ?? '',
  highlights: fromHighlights(page?.highlights ?? []),
  salesLed: page?.salesLed ?? false,
  showPrice: !page?.hidePrice,
  contactUrl: page?.contactUrl ?? '',
  productOrder: page?.productOrder ?? [],
  hiddenProductIds: page?.hiddenProductIds ?? [],
})

// The link and the hidden price only read for a sales-led plan, but are kept while it is not.
const payloadOf = (planId: string, draft: Draft) => ({
  planId,
  tagline: draft.tagline.trim() || undefined,
  popular: draft.popular,
  ctaLabel: draft.ctaLabel.trim() || undefined,
  highlights: toHighlights(draft.highlights),
  salesLed: draft.salesLed,
  hidePrice: !draft.showPrice,
  contactUrl: draft.contactUrl.trim() || undefined,
  productOrder: draft.productOrder,
  hiddenProductIds: draft.hiddenProductIds,
})

interface Feature {
  id: string
  name: string
  value: string
}

const Editor = ({
  planId,
  planVersionId,
  readOnly,
  onCancel,
  onSaved,
  actionsClassName,
  className,
  page,
  charges,
  familyId,
}: Props & { page?: PlanPricingPage; charges: PricingCharge[]; familyId?: string }) => {
  const queryClient = useQueryClient()
  // What the server holds; the form is dirty while the draft differs from it.
  const [saved, setSaved] = useState(() => JSON.stringify(payloadOf(planId, draftOf(page))))
  const [draft, setDraft] = useState(() => draftOf(page))
  const set = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft(current => ({ ...current, [key]: value }))
  const setHighlights = (update: (list: DraftHighlight[]) => DraftHighlight[]) =>
    setDraft(current => ({ ...current, highlights: update(current.highlights) }))

  const entitlements = useQuery(
    getResolvedEntitlementsForPlanVersion,
    planVersionId && env.entitlementsEnabled ? { planVersionId } : skipToken
  ).data?.entitlements
  // The resolved reference has no unit: the full features give "10,000 requests".
  const fullFeatures = useFeatures(
    useMemo(
      () => [...new Set((entitlements ?? []).flatMap(e => (e.feature ? [e.feature.id] : [])))],
      [entitlements]
    )
  )

  const features = new Map<string, Feature>()
  for (const entitlement of entitlements ?? []) {
    const feature = entitlement.feature
    if (!feature || features.has(feature.id) || isEntitlementDisabled(entitlement.value)) continue
    features.set(feature.id, {
      id: feature.id,
      name: feature.name,
      value: plainValue(
        create(EntitlementValueSchema, inheritedValue(entitlement)),
        fullFeatures.get(feature.id)
      ),
    })
  }

  const picked = new Set(
    draft.highlights.flatMap(h => (h.kind === 'feature' ? [h.featureId] : []))
  )
  const available = [...features.values()].filter(feature => !picked.has(feature.id))
  const full = draft.highlights.length >= MAX_HIGHLIGHTS

  const payload = payloadOf(planId, draft)
  const dirty = JSON.stringify(payload) !== saved
  // Nothing to order or leave off without a charge.
  const choosable = charges.length > 0

  // The card as the page reads it, its price as picked here: the saved copy with this draft's
  // price, rendered by the server.
  const pricedAs = {
    ...(JSON.parse(saved) as typeof payload),
    salesLed: payload.salesLed,
    hidePrice: payload.hidePrice,
    productOrder: payload.productOrder,
    hiddenProductIds: payload.hiddenProductIds,
  }
  const preview = useDebounceValue(JSON.stringify(pricedAs), 300)
  const previewPage = useQuery(
    getPricingPage,
    familyId && choosable
      ? {
          productFamilyId: familyId,
          draft: preview === saved ? undefined : (JSON.parse(preview) as typeof payload),
        }
      : skipToken,
    { placeholderData: keepPreviousData }
  ).data?.page
  const previewCard = previewPage?.cards.find(card => card.planId === planId)

  const update = useMutation(updatePlanPricingPage, {
    onSuccess: async data => {
      const next = draftOf(data.page)
      setDraft(next)
      setSaved(JSON.stringify(payloadOf(planId, next)))
      // Both the plan's fields and every product line's page read the same service.
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: getPricingPage.parent, cardinality: undefined }),
      })
      toast.success('Pricing page saved')
      onSaved?.()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The pricing page could not be saved')),
  })

  return (
    <form
      className={cn('flex flex-col gap-5', className)}
      onSubmit={event => {
        event.preventDefault()
        if (!readOnly && dirty) update.mutate({ page: payload })
      }}
    >
      <div className="space-y-1.5">
        <Label htmlFor={`pp-sold-${planId}`}>How it&apos;s sold</Label>
        <Select
          value={draft.salesLed ? 'sales' : 'self-serve'}
          disabled={readOnly}
          onValueChange={value => set('salesLed', value === 'sales')}
        >
          <SelectTrigger id={`pp-sold-${planId}`}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="self-serve">Self-serve</SelectItem>
            <SelectItem value="sales">Contact sales</SelectItem>
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">
          {draft.salesLed
            ? 'On the pricing page with a contact button. Not offered in the customer portal.'
            : 'Customers sign up and upgrade to it themselves.'}
        </p>
      </div>

      {draft.salesLed && (
        <>
          <div className="space-y-1.5">
            <Label htmlFor={`pp-contact-${planId}`}>Contact link</Label>
            <Input
              id={`pp-contact-${planId}`}
              type="url"
              value={draft.contactUrl}
              disabled={readOnly}
              placeholder="https://example.com/contact"
              onChange={e => set('contactUrl', e.target.value)}
            />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <Checkbox
              checked={draft.showPrice}
              disabled={readOnly}
              onCheckedChange={checked => set('showPrice', checked === true)}
            />
            Show the price
          </label>
        </>
      )}

      {choosable && !(draft.salesLed && !draft.showPrice) && (
        <CardOrder
          charges={charges}
          card={previewCard}
          currency={previewPage?.currency}
          productOrder={draft.productOrder}
          hiddenProductIds={draft.hiddenProductIds}
          readOnly={readOnly}
          onOrder={order => set('productOrder', order)}
          onShown={(productIds, shown) =>
            set(
              'hiddenProductIds',
              shown
                ? draft.hiddenProductIds.filter(id => !productIds.includes(id))
                : [...draft.hiddenProductIds, ...productIds]
            )
          }
        />
      )}

      <div className="space-y-1.5">
        <Label htmlFor={`pp-tagline-${planId}`}>Tagline</Label>
        <Input
          id={`pp-tagline-${planId}`}
          value={draft.tagline}
          disabled={readOnly}
          placeholder="For growing teams"
          onChange={e => set('tagline', e.target.value)}
        />
        <p className="text-xs text-muted-foreground">One line under the plan&apos;s name.</p>
      </div>

      <div className="space-y-1.5">
        <Label htmlFor={`pp-cta-${planId}`}>Button label</Label>
        <Input
          id={`pp-cta-${planId}`}
          value={draft.ctaLabel}
          disabled={readOnly}
          placeholder={draft.salesLed ? 'Contact sales' : 'Get started'}
          onChange={e => set('ctaLabel', e.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          {draft.salesLed ? (
            <>Empty: &ldquo;Contact sales&rdquo;.</>
          ) : (
            <>
              Empty: &ldquo;Get started&rdquo;, or &ldquo;Start a 14-day trial&rdquo; when the plan
              has a trial.
            </>
          )}
        </p>
      </div>

      <div className="flex items-start gap-3">
        <Switch
          id={`pp-popular-${planId}`}
          className="mt-0.5"
          checked={draft.popular}
          disabled={readOnly}
          onCheckedChange={checked => set('popular', checked)}
        />
        <div className="space-y-0.5">
          <Label htmlFor={`pp-popular-${planId}`}>Most popular</Label>
          <p className="text-xs text-muted-foreground">
            Unmarks the other plans of this product line.
          </p>
        </div>
      </div>

      <div className="space-y-2">
        <div className="flex items-baseline justify-between gap-2">
          <Label>Highlights</Label>
          <span className="text-xs tabular-nums text-muted-foreground">
            {draft.highlights.length} / {MAX_HIGHLIGHTS}
          </span>
        </div>
        <p className="text-xs text-muted-foreground">
          Listed on the card in this order. A feature shows its value in the published version.
        </p>

        {draft.highlights.length === 0 ? (
          <div className="rounded-md border border-dashed border-border px-4 py-5 text-center text-xs text-muted-foreground">
            No highlights yet. The card lists only what you add here.
          </div>
        ) : (
          <ol className="space-y-1.5">
            {draft.highlights.map((highlight, index) => (
              <HighlightRow
                key={highlight.key}
                highlight={highlight}
                feature={
                  highlight.kind === 'feature' ? features.get(highlight.featureId) : undefined
                }
                featuresLoaded={entitlements !== undefined}
                readOnly={readOnly}
                onText={text =>
                  setHighlights(list =>
                    list.map(h => (h.key === highlight.key ? { ...h, text } : h))
                  )
                }
                onUp={index > 0 ? () => setHighlights(list => moveItem(list, index, -1)) : undefined}
                onDown={
                  index < draft.highlights.length - 1
                    ? () => setHighlights(list => moveItem(list, index, 1))
                    : undefined
                }
                onRemove={() =>
                  setHighlights(list => list.filter(h => h.key !== highlight.key))
                }
              />
            ))}
          </ol>
        )}

        {!readOnly && (
          <div className="flex flex-wrap items-center gap-2">
            {planVersionId && env.entitlementsEnabled && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    hasIcon
                    disabled={full || available.length === 0}
                  >
                    <PlusIcon size={14} /> Add feature <ChevronDownIcon size={14} />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="max-h-72 overflow-y-auto">
                  <DropdownMenuLabel className="text-xs text-muted-foreground">
                    Features of this plan
                  </DropdownMenuLabel>
                  {available.map(feature => (
                    <DropdownMenuItem
                      key={feature.id}
                      onSelect={() =>
                        setHighlights(list => [
                          ...list,
                          { key: draftKey(), kind: 'feature', featureId: feature.id },
                        ])
                      }
                    >
                      <span className="truncate">{feature.name}</span>
                      <span className="ml-auto pl-4 text-xs text-muted-foreground">
                        {feature.value}
                      </span>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              hasIcon
              disabled={full}
              onClick={() =>
                setHighlights(list => [...list, { key: draftKey(), kind: 'text', text: '' }])
              }
            >
              <TypeIcon size={14} /> Add line
            </Button>
            {full && (
              <span className="text-xs text-muted-foreground">
                Up to {MAX_HIGHLIGHTS} highlights.
              </span>
            )}
          </div>
        )}
      </div>

      {!readOnly && (
        <div className={cn('flex justify-end gap-2', actionsClassName)}>
          {onCancel && (
            <Button type="button" variant="outline" onClick={onCancel}>
              Cancel
            </Button>
          )}
          <Button type="submit" disabled={!dirty || update.isPending}>
            Save
          </Button>
        </div>
      )}
    </form>
  )
}

const Hint = ({ text }: { text: string }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <button
        type="button"
        className="text-muted-foreground hover:text-foreground"
        aria-label={text}
      >
        <HelpCircleIcon size={13} />
      </button>
    </TooltipTrigger>
    <TooltipContent className="max-w-64 text-xs font-normal">{text}</TooltipContent>
  </Tooltip>
)

/** The order the card reads its charges in, the first that can lead being its price, and the
 * usage it leaves off: by product, kept across versions. */
const CardOrder = ({
  charges,
  card,
  currency,
  productOrder,
  hiddenProductIds,
  readOnly,
  onOrder,
  onShown,
}: {
  charges: PricingCharge[]
  /** The card as the server reads it with this draft, once the plan is on the page. */
  card?: PricingCard
  currency?: string
  productOrder: string[]
  hiddenProductIds: string[]
  readOnly?: boolean
  onOrder: (order: string[]) => void
  onShown: (productIds: string[], shown: boolean) => void
}) => {
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates })
  )
  const rows = inOrder(
    orderRows(card, charges, currency ?? '', hiddenProductIds),
    productOrder
  )
  const preview = card && currency ? priceDisplay(card, currency) : undefined
  const onDragEnd = ({ active, over }: DragEndEvent) => {
    if (!over || active.id === over.id) return
    const next = movedOrder(rows, productOrder, String(active.id), String(over.id))
    if (next !== productOrder) onOrder(next)
  }

  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-1.5">
        <Label>On the card</Label>
        <Hint text="The card reads its charges in this order: the first price leads the card, the others read below it. The base price, a seat or a usage rate can lead; one-time fees and credit packs never do. Hidden usage still reads as “+ usage”, so the card never looks all-in." />
      </div>
      <DndContext
        sensors={sensors}
        collisionDetection={closestCenter}
        modifiers={ORDER_MODIFIERS}
        onDragEnd={onDragEnd}
      >
        <SortableContext items={rows.map(row => row.id)} strategy={verticalListSortingStrategy}>
          <ul className="divide-y divide-border rounded-md border border-border">
            {rows.map(row => (
              <OrderItem key={row.id} row={row} readOnly={readOnly} onShown={onShown} />
            ))}
          </ul>
        </SortableContext>
      </DndContext>
      {preview?.kind === 'priced' && (
        <p className="text-xs text-muted-foreground">
          Reads {preview.from && 'from '}
          <span className="font-medium text-foreground">{preview.amount}</span> {preview.per}
          {preview.included && `, ${preview.included}`}
          {preview.extras.length > 0 && ` ${preview.extras.map(line => line.text).join(' · ')}`}
        </p>
      )}
    </div>
  )
}

// Rows move up and down their list only.
const ORDER_MODIFIERS = [restrictToVerticalAxis, restrictToParentElement]

const OrderItem = ({
  row,
  readOnly,
  onShown,
}: {
  row: OrderRow
  readOnly?: boolean
  onShown: (productIds: string[], shown: boolean) => void
}) => {
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition, isDragging } =
    useSortable({ id: row.id, disabled: readOnly })
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Translate.toString(transform), transition }}
      className={cn(
        'flex items-center gap-2 bg-card px-2 py-1.5 text-sm',
        isDragging && 'relative z-10 shadow-md'
      )}
    >
      {!readOnly && (
        <button
          type="button"
          ref={setActivatorNodeRef}
          {...attributes}
          {...listeners}
          aria-label={`Move ${row.name}`}
          className="cursor-grab touch-none rounded p-0.5 text-muted-foreground hover:text-foreground active:cursor-grabbing"
        >
          <GripVerticalIcon size={14} />
        </button>
      )}
      <span className={cn('min-w-0 shrink truncate', row.hidden && 'text-muted-foreground')}>
        {row.name}
      </span>
      <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{row.reading}</span>
      {row.lead && (
        <span className="shrink-0 rounded-full bg-brand/15 px-2 py-0.5 text-[11px] font-medium text-brand">
          Price
        </span>
      )}
      {row.hideable && (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0 text-muted-foreground"
          disabled={readOnly}
          aria-label={row.hidden ? `Show ${row.name}` : `Hide ${row.name}`}
          aria-pressed={row.hidden}
          onClick={() => onShown(row.productIds, row.hidden)}
        >
          {row.hidden ? <EyeOffIcon size={14} /> : <EyeIcon size={14} />}
        </Button>
      )}
    </li>
  )
}

const HighlightRow = ({
  highlight,
  feature,
  featuresLoaded,
  readOnly,
  onText,
  onUp,
  onDown,
  onRemove,
}: {
  highlight: DraftHighlight
  feature?: Feature
  featuresLoaded: boolean
  readOnly?: boolean
  onText: (text: string) => void
  onUp?: () => void
  onDown?: () => void
  onRemove: () => void
}) => (
  <li className="flex items-center gap-2 rounded-md border border-border bg-card px-2 py-1.5">
    <CheckIcon size={14} className="shrink-0 text-muted-foreground" aria-hidden />
    {highlight.kind === 'feature' ? (
      <div className="flex min-w-0 flex-1 items-baseline gap-2 px-1 py-1 text-sm">
        <span className="truncate font-medium">{feature?.name ?? 'Feature'}</span>
        <span className="truncate text-xs text-muted-foreground">
          {feature
            ? feature.value
            : featuresLoaded
              ? 'Not granted by this plan'
              : ''}
        </span>
      </div>
    ) : (
      <Input
        aria-label="Highlight"
        className="h-8 flex-1"
        value={highlight.text}
        maxLength={MAX_HIGHLIGHT_LENGTH}
        disabled={readOnly}
        placeholder="Priority support"
        onChange={e => onText(e.target.value)}
      />
    )}
    {!readOnly && (
      <div className="flex shrink-0 items-center">
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          aria-label="Move up"
          disabled={!onUp}
          onClick={onUp}
        >
          <ArrowUpIcon size={14} />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          aria-label="Move down"
          disabled={!onDown}
          onClick={onDown}
        >
          <ArrowDownIcon size={14} />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-7 w-7"
          aria-label="Remove"
          onClick={onRemove}
        >
          <XIcon size={14} />
        </Button>
      </div>
    )}
  </li>
)
