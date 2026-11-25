import { createConnectQueryKey, skipToken, useMutation, useQuery as useConnectQuery } from '@connectrpc/connect-query';
import { useQueryClient } from '@tanstack/react-query'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Form,
  MultiSelectFormField,
  MultiSelectItem,
  TextareaFormField,
} from '@ui/components'
import { ChevronDown } from 'lucide-react'
import { FunctionComponent, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { Gate } from '@/components/Gate'
import { LocalId } from '@/components/LocalId'
import { PanelHead, PanelSection } from '@/components/layouts'
import { EntityActivityTimeline } from '@/features/activity/EntityActivityTimeline'
import { fromDraft, isOpen, toDraft } from '@/features/eligibility/eligibility'
import {
  CouponDiscountField,
  CouponDurationField,
  MoreConditions,
} from '@/features/productCatalog/coupons/CouponTermsFields'
import { CouponStatusChip } from '@/features/productCatalog/coupons/CouponsTable'
import {
  couponDate,
  fixedDiscountOf,
  hasCouponTerms,
  otherAmountsOf,
  parseCouponDate,
} from '@/features/productCatalog/coupons/couponTerms'
import {
  couponAppliesTo,
  couponDiscountLine,
  couponStatus,
  couponWhoLine,
} from '@/features/productCatalog/coupons/couponWords'
import { getCountryName } from '@/features/settings/utils'
import { useQueryState } from '@/hooks/useQueryState'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { schemas } from '@/lib/schemas'
import {
  editCoupon,
  getCoupon,
  listCoupons,
  removeCoupon,
  updateCouponStatus,
} from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { CouponAction } from '@/rpc/api/coupons/v1/coupons_pb'
import { CouponDiscount_CreditScope } from '@/rpc/api/coupons/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { parseAndFormatDateOptional } from '@/utils/date'

import type { CouponTermsFormFields } from '@/features/productCatalog/coupons/CouponTermsFields'
import type { Control } from 'react-hook-form'

/** The coupon open beside the list: its terms in words, the short form, its activity. */
export const CouponDetails: FunctionComponent<{ couponLocalId: string; onClose: () => void }> = ({
  couponLocalId,
  onClose,
}) => {
  const queryClient = useQueryClient()

  const query = useQuery(
    getCoupon,
    couponLocalId
      ? {
          couponLocalId: couponLocalId,
        }
      : skipToken
  )

  const invalidate = async () => {
    await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: listCoupons.parent,
      cardinality: undefined
    }) })
  }

  const editCouponMut = useMutation(editCoupon, {
    onSuccess: async () => {
      await invalidate()
      toast.success('Coupon updated')
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The coupon could not be updated')),
  })

  const removeCouponMut = useMutation(removeCoupon, {
    onSuccess: async () => {
      await invalidate()
      onClose()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The coupon could not be removed')),
  })

  const updateCouponStatusMut = useMutation(updateCouponStatus, {
    onSuccess: invalidate,
    onError: error => toast.error(rpcErrorMessage(error, 'The coupon status could not be changed')),
  })

  const plansQuery = useConnectQuery(listPlans, { pagination: { page: 0, perPage: 100 } })
  const [eligibility, setEligibility] = useState(() => toDraft(undefined))

  const [, setTab] = useQueryState<string>('filter', '')

  const methods = useZodForm({
    schema: schemas.coupons.editComponentSchema,
    defaultValues: {
      description: '',
      discountType: 'percentage',
      percentage: undefined,
      planIds: [],
      addOnIds: [],
      otherAmounts: [],
      duration: undefined,
      durationMonths: undefined,
    },
  })
  const termsControl = methods.control as unknown as Control<CouponTermsFormFields>

  const onSubmit = async (data: z.infer<typeof schemas.coupons.editComponentSchema>) => {
    // Existing redemptions keep the months they were made with: this reaches later ones.
    const months =
      data.discountType === 'credit' || data.duration === undefined
        ? {}
        : data.duration === 'months'
          ? { durationMonths: data.durationMonths }
          : { clearDurationMonths: true }
    editCouponMut.mutateAsync({
      ...months,
      couponId: query.data?.coupon?.id,
      description: data.description,
      discount: {
        discountType:
          data.discountType === 'fixed'
            ? {
                case: 'fixed' as const,
                value: fixedDiscountOf(data.amount ?? '', data.currency, data.otherAmounts),
              }
            : data.discountType === 'credit'
              ? {
                  case: 'credit' as const,
                  value: {
                    amount: data.creditAmount,
                    currency: data.currency,
                    expiresInDays: data.creditExpiresInDays,
                    scope: data.creditSubscriptionOnly
                      ? CouponDiscount_CreditScope.SUBSCRIPTION
                      : CouponDiscount_CreditScope.CUSTOMER,
                  },
                }
              : {
                  case: 'percentage' as const,
                  value: { percentage: data.percentage },
                },
      },
      planIds: data.planIds ?? [],
      addOnIds: data.addOnIds ?? [],
      // Left out keeps a date: an empty one clears it.
      validFrom: data.validFrom ? couponDate(data.validFrom) : coupon?.validFrom ? '' : undefined,
      expiresAt: data.expiresAt ? couponDate(data.expiresAt) : coupon?.expiresAt ? '' : undefined,
      // An empty rule removes it: anyone holding the code redeems.
      eligibility: fromDraft(eligibility),
    })
  }

  const discountType = methods.watch('discountType')
  const duration = methods.watch('duration')

  const coupon = query.data?.coupon
  // The server rejects switching a coupon between promotional credit and a discount.
  const isCreditCoupon = coupon?.discount?.discountType?.case === 'credit'

  // Reset form when coupon data loads or changes
  useEffect(() => {
    if (!coupon) return

    setEligibility(toDraft(coupon.eligibility))
    const terms = {
      addOnIds: coupon.addOnIds ?? [],
      validFrom: parseCouponDate(coupon.validFrom),
      expiresAt: parseCouponDate(coupon.expiresAt),
    }
    const kind = coupon.discount?.discountType
    if (kind?.case === 'credit') {
      methods.reset({
        ...terms,
        description: coupon.description,
        discountType: 'credit',
        creditAmount: kind.value.amount,
        currency: kind.value.currency,
        creditExpiresInDays: kind.value.expiresInDays,
        creditSubscriptionOnly: kind.value.scope === CouponDiscount_CreditScope.SUBSCRIPTION,
        planIds: coupon.planIds ?? [],
      })
      return
    }

    methods.reset({
      ...terms,
      // A coupon counting invoices, made before months, keeps counting them.
      duration:
        coupon.recurringValue === undefined
          ? coupon.durationMonths
            ? 'months'
            : 'forever'
          : undefined,
      durationMonths: coupon.durationMonths,
      otherAmounts: kind?.case === 'fixed' ? otherAmountsOf(kind.value) : [],
      amount:
        coupon.discount?.discountType?.case === 'fixed'
          ? coupon.discount.discountType.value.amount
          : undefined,
      currency:
        coupon.discount?.discountType?.case === 'fixed'
          ? coupon.discount.discountType.value.currency
          : '',
      description: coupon.description,
      discountType: kind?.case,
      percentage:
        coupon.discount?.discountType?.case === 'percentage'
          ? coupon.discount.discountType.value.percentage
          : undefined,
      planIds: coupon.planIds ?? [],
    })
  }, [coupon?.id])

  if (!coupon) {
    return <div className="h-40 animate-pulse p-4" />
  }
  const kind = coupon.discount?.discountType
  const fixed = kind?.case === 'fixed' ? kind.value : undefined
  const credit = kind?.case === 'credit' ? kind.value : undefined
  const termsSet = hasCouponTerms({
    validFrom: parseCouponDate(coupon.validFrom),
    expiresAt: parseCouponDate(coupon.expiresAt),
    otherAmounts: fixed ? otherAmountsOf(fixed) : [],
    creditExpiresInDays: credit?.expiresInDays,
    creditSubscriptionOnly: credit?.scope === CouponDiscount_CreditScope.SUBSCRIPTION,
    addOnCount: coupon.addOnIds.length,
    eligibility: isOpen(coupon.eligibility) ? undefined : coupon.eligibility,
  })

  // Named while few, counted otherwise.
  const planNames = coupon.planIds.flatMap(
    id => plansQuery.data?.plans.find(p => p.id === id)?.name ?? []
  )
  const appliesTo =
    planNames.length && planNames.length === coupon.planIds.length && planNames.length <= 3
      ? [
          planNames.join(', '),
          coupon.addOnIds.length && couponAppliesTo({ planIds: [], addOnIds: coupon.addOnIds }),
        ]
          .filter(Boolean)
          .join(' · ')
      : couponAppliesTo(coupon)

  const actions = (
    <Gate perm="catalog:manage">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="outline" size="sm" hasIcon className="h-7">
            Actions <ChevronDown className="w-4 h-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {coupon.archivedAt !== undefined ? (
            <DropdownMenuItem
              key="enable"
              onClick={() =>
                updateCouponStatusMut
                  .mutateAsync({ action: CouponAction.ENABLE, couponId: coupon.id })
                  .then(() => setTab('active'))
              }
            >
              Restore & enable
            </DropdownMenuItem>
          ) : coupon.disabled ? (
            <DropdownMenuItem
              key="enable"
              onClick={() =>
                updateCouponStatusMut
                  .mutateAsync({ action: CouponAction.ENABLE, couponId: coupon.id })
                  .then(() => setTab('active'))
              }
            >
              Enable
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem
              key="disable"
              onClick={() =>
                updateCouponStatusMut
                  .mutateAsync({ action: CouponAction.DISABLE, couponId: coupon.id })
                  .then(() => setTab('inactive'))
              }
            >
              Disable
            </DropdownMenuItem>
          )}

          {coupon.archivedAt ? null : (
            <DropdownMenuItem
              key="archive"
              onClick={() =>
                updateCouponStatusMut
                  .mutateAsync({ action: CouponAction.ARCHIVE, couponId: coupon.id })
                  .then(() => setTab('archived'))
              }
            >
              Archive
            </DropdownMenuItem>
          )}

          {coupon.lastRedemptionAt ? null : (
            <DropdownMenuItem
              key="delete"
              onClick={() => removeCouponMut.mutateAsync({ couponId: coupon.id })}
              className="bg-destructive text-destructive-foreground"
            >
              Delete
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </Gate>
  )

  return (
    <div className="divide-y divide-border">
      <PanelHead
        title={
          <div className="flex items-center gap-2">
            <CopyToClipboardButton
              text={coupon.code}
              buttonVariant="ghost"
              buttonClassName="h-7 px-1.5 font-mono text-base font-semibold"
            />
            <CouponStatusChip status={couponStatus(coupon)} />
          </div>
        }
        actions={actions}
        onClose={onClose}
      />

      <dl className="grid grid-cols-[8rem_1fr] gap-x-3 gap-y-1.5 px-4 py-3 text-sm">
        <dt className="text-muted-foreground">Discount</dt>
        <dd>{couponDiscountLine(coupon)}</dd>
        <dt className="text-muted-foreground">Who can redeem it</dt>
        <dd>{couponWhoLine(coupon, code => getCountryName(code))}</dd>
        <dt className="text-muted-foreground">Applies to</dt>
        <dd>{appliesTo}</dd>
        <dt className="text-muted-foreground">Redeemed</dt>
        <dd className="tabular-nums">
          {coupon.redemptionCount} / {coupon.redemptionLimit ?? '∞'}
          {coupon.lastRedemptionAt && (
            <span className="text-muted-foreground">
              {' '}
              · last {parseAndFormatDateOptional(coupon.lastRedemptionAt)}
            </span>
          )}
        </dd>
        <dt className="text-muted-foreground">Per customer</dt>
        <dd>{isCreditCoupon ? 'Once' : coupon.reusable ? 'Reusable' : 'Once'}</dd>
        <dt className="text-muted-foreground">API handle</dt>
        <dd>
          <LocalId localId={coupon.localId} className="max-w-24" />
        </dd>
      </dl>

      <Form {...methods}>
        <form className="space-y-5 px-4 py-4" onSubmit={methods.handleSubmit(onSubmit)}>
          <TextareaFormField
            name="description"
            label="Description"
            control={methods.control}
            placeholder="YC deal: 30% off for the first 6 months"
          />

          <CouponDiscountField
            control={termsControl}
            locked={isCreditCoupon ? ['percentage', 'fixed'] : ['credit']}
          />

          {discountType !== 'credit' && duration !== undefined && (
            <CouponDurationField control={termsControl} note="Existing redemptions keep theirs." />
          )}

          <MultiSelectFormField
            name="planIds"
            label="Plans"
            control={methods.control}
            placeholder="All plans"
            hasSearch
          >
            {plansQuery.data?.plans?.map(plan => (
              <MultiSelectItem key={plan.id} value={plan.id}>
                {plan.name}
              </MultiSelectItem>
            ))}
          </MultiSelectFormField>

          <MoreConditions
            key={coupon.id}
            control={termsControl}
            kind={discountType}
            eligibility={eligibility}
            onEligibilityChange={setEligibility}
            defaultOpen={termsSet}
          />
          <Gate perm="catalog:manage">
            <div className="flex justify-end">
              <Button type="submit" variant="primary" size="sm" disabled={editCouponMut.isPending}>
                Save
              </Button>
            </div>
          </Gate>
        </form>
      </Form>

      <PanelSection title="Activity">
        <EntityActivityTimeline
          entityType="coupon"
          entityId={coupon.id}
          emptyLabel="No activity yet for this coupon"
        />
      </PanelSection>
    </div>
  )
}
