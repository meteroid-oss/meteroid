import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  Label,
  RadioGroup,
  RadioGroupItem,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useAtom } from 'jotai'
import { ArrowRight, Calendar, Minus, Pencil, Plus, Zap } from 'lucide-react'
import { useEffect, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { feeTypeEnumToComponentFeeType } from '@/features/plans/addons/AddOnCard'
import { priceSummaryBadges } from '@/features/plans/pricecomponents/utils'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  formDataToPrice,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing'
import { InvoicePreviewCard } from '@/features/subscriptions/UpcomingInvoiceCard'
import {
  formatSubscriptionFeeBillingPeriod,
  formatSubscriptionFeeCompact,
} from '@/features/subscriptions/utils/fees'
import { useQuery } from '@/lib/connectrpc'
import { priceToSubscriptionFee } from '@/lib/mapping/priceToSubscriptionFee'
import { formatCurrency } from '@/lib/utils/numbers'
import { amendSubscriptionAtom } from '@/pages/tenants/subscription/amend/state'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import {
  CreateSubscriptionAddOn,
  CreateSubscriptionAddOn_AddOnPriceOverride,
} from '@/rpc/api/subscriptions/v1/models_pb'
import {
  applyAmendment,
  getSubscriptionDetails,
  previewAmendment,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import {
  AmendmentAddComponent,
  AmendmentAddOnChanges,
  AmendmentComponentChanges,
  AmendmentEditAddOn,
  AmendmentEditComponent,
  PlanChangeApplyMode,
} from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import { parseAndFormatDate } from '@/utils/date'

import type {
  AmendSubscriptionState,
  EditComponentDraft,
  ExtraComponentDraft,
} from '@/pages/tenants/subscription/amend/state'
import type { PartialMessage } from '@bufbuild/protobuf'

const buildPriceEntry = (
  feeType: EditComponentDraft['feeType'],
  formData: Record<string, unknown>,
  currency: string
) => {
  const pricingType = toPricingTypeFromFeeType(
    feeType,
    feeType === 'usage' ? (formData.usageModel as string) : undefined
  )
  return wrapAsNewPriceEntries(buildPriceInputs(pricingType, formData, currency))[0]
}

const buildComponentChanges = (
  state: AmendSubscriptionState,
  currency: string
): PartialMessage<AmendmentComponentChanges> => ({
  edited: state.componentChanges.edited.map(
    (c): AmendmentEditComponent =>
      new AmendmentEditComponent({
        subscriptionComponentId: c.subscriptionComponentId,
        name: c.name,
        price: buildPriceEntry(c.feeType, c.formData, currency),
      })
  ),
  added: state.componentChanges.added.map(
    (c: ExtraComponentDraft): AmendmentAddComponent =>
      new AmendmentAddComponent({
        name: c.name,
        product: c.productId
          ? buildExistingProductRef(c.productId)
          : buildNewProductRef(c.name, c.feeType, c.formData),
        price: buildPriceEntry(c.feeType, c.formData, currency),
      })
  ),
  removedComponentIds: state.componentChanges.removedComponentIds,
})

const buildAddOnChanges = (
  state: AmendSubscriptionState,
  currency: string
): PartialMessage<AmendmentAddOnChanges> => ({
  added: state.addOnChanges.added.map(
    (a): CreateSubscriptionAddOn =>
      new CreateSubscriptionAddOn({ addOnId: a.addOnId, quantity: a.quantity })
  ),
  edited: state.addOnChanges.edited.map((a): AmendmentEditAddOn => {
    const base = { subscriptionAddOnId: a.subscriptionAddOnId, quantity: a.quantity }
    if (a.priceOverride) {
      return new AmendmentEditAddOn({
        ...base,
        customization: {
          case: 'priceOverride',
          value: new CreateSubscriptionAddOn_AddOnPriceOverride({
            priceEntry: buildPriceEntry(a.priceOverride.feeType, a.priceOverride.formData, currency),
          }),
        },
      })
    }
    return new AmendmentEditAddOn(base)
  }),
  removedAddOnIds: state.addOnChanges.removedAddOnIds,
})

const formatNewFee = (
  feeType: EditComponentDraft['feeType'],
  formData: Record<string, unknown>,
  currency: string
): string => {
  try {
    return formatSubscriptionFeeCompact(
      priceToSubscriptionFee(formDataToPrice(feeType, formData, currency)),
      currency
    )
  } catch {
    return ''
  }
}

type ChangeItem = {
  kind: 'add' | 'remove' | 'edit'
  label: string
  name: string
  period?: string
  /** Plain detail line (e.g. the price of an added item). */
  detail?: string
  /** Old value, shown struck-through, for edits (price or quantity). */
  before?: string
  /** New value, shown highlighted, for edits. */
  after?: string
}

// Connect/gRPC errors surface as "[code] message"; drop the machine code prefix
// so the user sees a readable reason (e.g. the max-instances validation error).
const cleanRpcMessage = (error: unknown): string => {
  const raw = error instanceof Error ? error.message : 'Could not compute the preview'
  return raw.replace(/^\[[a-z_]+\]\s*/i, '')
}

const termToPeriodLabel = (term: unknown): string | undefined => {
  switch (term) {
    case 'MONTHLY':
      return 'Monthly'
    case 'QUARTERLY':
      return 'Quarterly'
    case 'SEMIANNUAL':
      return 'Semiannual'
    case 'ANNUAL':
      return 'Yearly'
    default:
      return undefined
  }
}

const isEmptyAmendment = (state: AmendSubscriptionState): boolean =>
  state.componentChanges.edited.length === 0 &&
  state.componentChanges.added.length === 0 &&
  state.componentChanges.removedComponentIds.length === 0 &&
  state.addOnChanges.added.length === 0 &&
  state.addOnChanges.edited.length === 0 &&
  state.addOnChanges.removedAddOnIds.length === 0

export const StepReviewApply = () => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { previousStep } = useWizard()
  const [state, setState] = useAtom(amendSubscriptionAtom)

  const previewMut = useMutation(previewAmendment)
  const applyMut = useMutation(applyAmendment)

  const currency = state.currency || 'USD'
  const isImmediate = state.applyMode === PlanChangeApplyMode.IMMEDIATE
  const isEmpty = isEmptyAmendment(state)

  // Live components/add-ons + add-on catalog, to resolve names for the change list.
  const subQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId: state.subscriptionId },
    { enabled: Boolean(state.subscriptionId) }
  )
  const catalogQuery = useQuery(listAddOns, { pagination: { perPage: 100, page: 0 } })

  const componentById = new Map((subQuery.data?.priceComponents ?? []).map(c => [c.id, c]))
  const componentNameById = new Map(
    (subQuery.data?.priceComponents ?? []).map(c => [c.id, c.name])
  )
  const addOnById = new Map((subQuery.data?.addOns ?? []).map(a => [a.id, a]))
  const addOnNameById = new Map((subQuery.data?.addOns ?? []).map(a => [a.id, a.name]))
  const catalogById = new Map((catalogQuery.data?.addOns ?? []).map(c => [c.id, c]))

  const componentPeriod = (id: string): string | undefined => {
    const c = componentById.get(id)
    return c ? formatSubscriptionFeeBillingPeriod(c.period) : undefined
  }
  const addOnPeriod = (id: string): string | undefined => {
    const a = addOnById.get(id)
    return a ? formatSubscriptionFeeBillingPeriod(a.period) : undefined
  }

  // The change list is built from the user's actual edits (not the backend's
  // remove+add proration decomposition), so a price override reads as "Price
  // updated" rather than a paired removal + addition.
  const changeRows: ChangeItem[] = [
    // Component price override: old fee → new fee (same as the editor row).
    ...state.componentChanges.edited.map(c => {
      const live = componentById.get(c.subscriptionComponentId)
      return {
        kind: 'edit' as const,
        label: 'Price updated',
        name: c.name || componentNameById.get(c.subscriptionComponentId) || 'Component',
        before: live ? formatSubscriptionFeeCompact(live.fee, currency) : undefined,
        after: formatNewFee(c.feeType, c.formData, currency),
        period: componentPeriod(c.subscriptionComponentId),
      }
    }),
    ...state.componentChanges.added.map(c => ({
      kind: 'add' as const,
      label: 'Component added',
      name: c.name,
      detail: formatNewFee(c.feeType, c.formData, currency),
      period: termToPeriodLabel(c.formData.term),
    })),
    ...state.componentChanges.removedComponentIds.map(id => {
      const live = componentById.get(id)
      return {
        kind: 'remove' as const,
        label: 'Component removed',
        name: componentNameById.get(id) ?? 'Component',
        detail: live ? formatSubscriptionFeeCompact(live.fee, currency) : undefined,
        period: componentPeriod(id),
      }
    }),
    ...state.addOnChanges.added.map(a => {
      const cat = catalogById.get(a.addOnId)
      const price = cat
        ? priceSummaryBadges(feeTypeEnumToComponentFeeType(cat.feeType), cat.price, currency).join(
            ' / '
          )
        : ''
      const parts = [a.quantity > 1 ? `×${a.quantity}` : '', price].filter(Boolean)
      return {
        kind: 'add' as const,
        label: 'Add-on added',
        name: cat?.name ?? 'Add-on',
        detail: parts.join(' · ') || undefined,
      }
    }),
    ...state.addOnChanges.edited.map(a => {
      const live = addOnById.get(a.subscriptionAddOnId)
      const livePrice = live ? formatSubscriptionFeeCompact(live.fee, currency) : undefined
      // Price override: old → new fee. Quantity change: old → new count (with the
      // unit price kept as plain detail, mirroring the editor row).
      if (a.priceOverride) {
        return {
          kind: 'edit' as const,
          label: 'Price updated',
          name: a.name || addOnNameById.get(a.subscriptionAddOnId) || 'Add-on',
          before: livePrice,
          after: formatNewFee(a.priceOverride.feeType, a.priceOverride.formData, currency),
          period: addOnPeriod(a.subscriptionAddOnId),
        }
      }
      return {
        kind: 'edit' as const,
        label: 'Quantity updated',
        name: a.name || addOnNameById.get(a.subscriptionAddOnId) || 'Add-on',
        before: live ? `×${live.quantity}` : undefined,
        after: a.quantity != null ? `×${a.quantity}` : undefined,
        detail: livePrice,
        period: addOnPeriod(a.subscriptionAddOnId),
      }
    }),
    ...state.addOnChanges.removedAddOnIds.map(id => {
      const live = addOnById.get(id)
      return {
        kind: 'remove' as const,
        label: 'Add-on removed',
        name: addOnNameById.get(id) ?? 'Add-on',
        detail: live ? formatSubscriptionFeeCompact(live.fee, currency) : undefined,
        period: addOnPeriod(id),
      }
    }),
  ]

  const buildRequest = () => ({
    subscriptionId: state.subscriptionId,
    applyMode: state.applyMode,
    componentChanges: buildComponentChanges(state, currency),
    addOnChanges: buildAddOnChanges(state, currency),
  })

  // Re-run preview whenever the deltas or apply mode change.
  // A serialized signature of the deltas keeps the effect honest without
  // re-running on unrelated state updates (e.g. the preview itself).
  const deltaSignature = useMemo(
    () => JSON.stringify({ c: state.componentChanges, a: state.addOnChanges, m: state.applyMode }),
    [state.componentChanges, state.addOnChanges, state.applyMode]
  )

  useEffect(() => {
    if (!state.subscriptionId) return
    // No changes (or all changes reset back to their originals): clear any stale
    // preview instead of previewing a zero-sum amendment.
    if (isEmpty) {
      setState(prev => (prev.preview ? { ...prev, preview: undefined } : prev))
      previewMut.reset()
      return
    }
    previewMut.mutate(buildRequest(), {
      onSuccess: data => setState(prev => ({ ...prev, preview: data })),
      // Drop the stale preview on failure so the error banner isn't shown next
      // to outdated numbers.
      onError: () => setState(prev => (prev.preview ? { ...prev, preview: undefined } : prev)),
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deltaSignature, state.subscriptionId])

  const previewError = previewMut.isError ? cleanRpcMessage(previewMut.error) : null

  const preview = state.preview

  const handleModeChange = (value: string) => {
    const mode =
      value === 'immediate' ? PlanChangeApplyMode.IMMEDIATE : PlanChangeApplyMode.END_OF_PERIOD
    setState(prev => ({ ...prev, applyMode: mode }))
  }

  const handleApply = async () => {
    if (!state.subscriptionId || isEmpty) return
    try {
      const result = await applyMut.mutateAsync(buildRequest())
      if (isImmediate) {
        toast.success(
          `Amendment applied immediately${result.invoiceId ? ' — adjustment invoice created' : ''}`
        )
      } else {
        toast.success(
          `Amendment scheduled for ${result.effectiveDate ? parseAndFormatDate(result.effectiveDate) : 'end of current period'}`
        )
      }
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getSubscriptionDetails, {
          subscriptionId: state.subscriptionId,
        }),
      })
      navigate(`../${state.subscriptionId}`, { replace: true })
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Failed to apply amendment'
      toast.error(message)
    }
  }

  return (
    <div className="space-y-6">
      <PageSection
        header={{
          title: 'Review & apply',
          subtitle: 'Review the amendment and choose when to apply',
        }}
      >
        <div className="space-y-6">
          {/* Apply mode selector */}
          <Card>
            <CardHeader className="flex flex-row items-center gap-2">
              <Zap className="h-5 w-5" />
              <CardTitle className="text-base">When to apply</CardTitle>
            </CardHeader>
            <CardContent>
              <RadioGroup
                value={isImmediate ? 'immediate' : 'end_of_period'}
                onValueChange={handleModeChange}
                className="space-y-3"
              >
                <div className="flex items-start gap-3">
                  <RadioGroupItem value="end_of_period" id="mode-eop" className="mt-0.5" />
                  <Label htmlFor="mode-eop" className="cursor-pointer">
                    <div className="text-sm font-medium">At end of billing period</div>
                    <div className="text-xs text-muted-foreground">
                      The amendment will take effect at the next renewal date.
                    </div>
                  </Label>
                </div>
                <div className="flex items-start gap-3">
                  <RadioGroupItem value="immediate" id="mode-immediate" className="mt-0.5" />
                  <Label htmlFor="mode-immediate" className="cursor-pointer">
                    <div className="text-sm font-medium">Immediately</div>
                    <div className="text-xs text-muted-foreground">
                      Apply now with a prorated adjustment invoice.
                    </div>
                  </Label>
                </div>
              </RadioGroup>

              {/* Effective date — a sub-section of "When to apply". */}
              {preview?.effectiveDate && (
                <div className="mt-4 pt-3 border-t border-border flex items-center gap-2 text-sm">
                  <Calendar className="h-4 w-4 text-muted-foreground shrink-0" />
                  <span className="text-muted-foreground">Effective</span>
                  <span className="font-medium text-foreground">
                    {parseAndFormatDate(preview.effectiveDate)}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {isImmediate ? '(applied immediately)' : '(at end of current period)'}
                  </span>
                </div>
              )}
            </CardContent>
          </Card>

          {/* Changes — placed above the proration so the user sees what changed first.
              Driven by the user's edits; MRR is folded in as a small footer. */}
          {changeRows.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Changes</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="divide-y divide-border">
                  {changeRows.map((row, idx) => (
                    <ChangeRow
                      key={`${row.kind}-${idx}`}
                      kind={row.kind}
                      name={row.name}
                      detail={row.detail}
                      before={row.before}
                      after={row.after}
                      period={row.period}
                      label={row.label}
                    />
                  ))}
                </div>
                {preview?.mrr && (
                  <div className="mt-3 pt-3 border-t border-border flex items-center gap-2 text-xs text-muted-foreground">
                    <span>MRR</span>
                    <span>{formatCurrency(Number(preview.mrr.beforeCents), currency)}</span>
                    <ArrowRight className="h-3 w-3" />
                    <span className="font-medium text-foreground">
                      {formatCurrency(Number(preview.mrr.afterCents), currency)}
                    </span>
                    {preview.mrr.deltaCents !== 0n && (
                      <span className={preview.mrr.deltaCents > 0n ? 'text-brand' : 'text-destructive'}>
                        ({preview.mrr.deltaCents > 0n ? '+' : ''}
                        {formatCurrency(Number(preview.mrr.deltaCents), currency)}/mo)
                      </span>
                    )}
                  </div>
                )}
              </CardContent>
            </Card>
          )}

          {/* Preview error (e.g. an add-on quantity exceeding its max) */}
          {previewError && !isEmpty && (
            <div className="rounded-lg border border-destructive/40 bg-destructive/5 p-4 text-sm">
              <div className="font-medium text-destructive">Could not compute the preview</div>
              <div className="mt-1 text-destructive/90">{previewError}</div>
            </div>
          )}

          {/* Proration preview (immediate only) */}
          {isImmediate && preview?.proration && (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Proration Summary</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="space-y-2 text-sm">
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Credit (removed / replaced items)</span>
                    <span className="text-foreground">
                      {formatCurrency(preview.proration.creditsTotalCents, currency)}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Charge (added / replaced items)</span>
                    <span className="text-foreground">
                      {formatCurrency(preview.proration.chargesTotalCents, currency)}
                    </span>
                  </div>
                  <div className="border-t border-border my-2" />
                  <div className="flex justify-between font-medium">
                    <span className="text-foreground">Net adjustment</span>
                    <span className="text-foreground">
                      {formatCurrency(preview.proration.netAmountCents, currency)}
                    </span>
                  </div>
                  <div className="text-xs text-muted-foreground mt-2">
                    {preview.proration.daysRemaining} of {preview.proration.daysInPeriod} days
                    remaining in current period (
                    {Math.round(preview.proration.prorationFactor * 100)}% prorated)
                  </div>
                </div>
              </CardContent>
            </Card>
          )}

          {/* Adjustment invoice (immediate) and next-cycle invoice line-item previews */}
          {isImmediate && preview?.adjustmentInvoice && (
            <InvoicePreviewCard
              invoice={preview.adjustmentInvoice}
              currency={currency}
              subscriptionId={state.subscriptionId}
              title="Adjustment invoice (charged now)"
            />
          )}
          {isImmediate && preview?.creditNote && (
            <InvoicePreviewCard
              invoice={preview.creditNote}
              currency={currency}
              subscriptionId={state.subscriptionId}
              title="Credit note (issued now)"
            />
          )}
          {/* A credit is genuinely owed but there is no finalized current-period
              invoice to credit against (e.g. it is still a draft, or the period is
              billed in arrears), so no credit note can be issued. Gate on the netted
              credit, not the gross credit line: a price override nets its credit into
              its charge, so an upgrade has no credit owed even though the gross credit
              is negative. */}
          {isImmediate &&
            !preview?.creditNote &&
            preview?.proration &&
            preview.proration.netCreditCents < 0n && (
              <div className="rounded-lg border border-border p-4 text-sm text-muted-foreground">
                A credit of{' '}
                <span className="font-medium text-foreground">
                  {formatCurrency(Number(-preview.proration.netCreditCents), currency)}
                </span>{' '}
                applies to this change, but there is no finalized invoice for the current
                period to credit against, so no credit note will be issued. It is reflected in
                the net adjustment above.
              </div>
            )}
          {/* A deferred arrears charge means one or more newly-added components bill
              in arrears: the prorated amount lands on the next renewal invoice, not
              now. Show a note whether or not there is also an immediate invoice. */}
          {isImmediate &&
            preview?.proration &&
            preview.proration.arrearsChargeCents > 0n && (
              <div className="rounded-lg border border-border p-4 text-sm text-muted-foreground">
                {preview.adjustmentInvoice
                  ? 'Additionally, a'
                  : 'This change adds a'}{' '}
                prorated arrears charge of{' '}
                <span className="font-medium text-foreground">
                  {formatCurrency(Number(preview.proration.arrearsChargeCents), currency)}
                </span>{' '}
                that will be included in the next renewal invoice rather than charged now.
              </div>
            )}
          {preview?.nextInvoice && (
            <InvoicePreviewCard
              invoice={preview.nextInvoice}
              currency={currency}
              subscriptionId={state.subscriptionId}
              title="Next renewal invoice"
            />
          )}

          {isEmpty && (
            <div className="rounded-lg border border-border p-4 text-sm text-muted-foreground">
              No changes yet. Go back to edit components or add-ons.
            </div>
          )}
        </div>
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="secondary" onClick={previousStep} disabled={applyMut.isPending}>
          Back
        </Button>
        <Button
          variant="brand"
          onClick={handleApply}
          disabled={applyMut.isPending || isEmpty || Boolean(previewError)}
          className="min-w-[180px]"
        >
          {applyMut.isPending
            ? isImmediate
              ? 'Applying...'
              : 'Scheduling...'
            : isImmediate
              ? 'Apply Amendment Now'
              : 'Schedule Amendment'}
        </Button>
      </div>
    </div>
  )
}

const ChangeRow = ({
  kind,
  name,
  detail,
  before,
  after,
  period,
  label,
}: {
  kind: 'add' | 'remove' | 'edit'
  name: string
  detail?: string
  before?: string
  after?: string
  period?: string
  label: string
}) => {
  // The secondary line mirrors the editor: an old → new transition for edits,
  // a plain detail otherwise, with the billing period appended.
  const hasTransition = Boolean(before || after)
  return (
    <div className="flex items-center justify-between gap-3 py-3">
      <div className="flex items-center gap-2 min-w-0">
        {kind === 'add' ? (
          <Plus className="h-4 w-4 text-brand shrink-0" />
        ) : kind === 'remove' ? (
          <Minus className="h-4 w-4 text-destructive shrink-0" />
        ) : (
          <Pencil className="h-4 w-4 text-brand shrink-0" />
        )}
        <div className="min-w-0">
          <div className={`text-sm font-medium ${kind === 'remove' ? 'line-through' : ''}`}>
            {name}
          </div>
          {(hasTransition || detail || period) && (
            <div className="text-xs text-muted-foreground flex items-center gap-1 flex-wrap">
              {hasTransition && (
                <>
                  {before && <span className="line-through opacity-60">{before}</span>}
                  {before && after && <ArrowRight className="h-3 w-3" />}
                  {after && <span className="text-brand">{after}</span>}
                </>
              )}
              {detail && (
                <span>
                  {hasTransition ? ' · ' : ''}
                  {detail}
                </span>
              )}
              {period && (
                <span className="opacity-70">
                  {hasTransition || detail ? ' · ' : ''}
                  {period}
                </span>
              )}
            </div>
          )}
        </div>
      </div>
      <Badge
        variant={kind === 'remove' ? 'destructive' : 'outline'}
        size="sm"
        className={kind === 'remove' ? '' : 'text-brand border-brand/30'}
      >
        <span className="flex items-center gap-1">
          <ArrowRight className="h-3 w-3" />
          {label}
        </span>
      </Badge>
    </div>
  )
}
