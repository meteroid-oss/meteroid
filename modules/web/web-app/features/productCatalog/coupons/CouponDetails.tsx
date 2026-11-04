import { createConnectQueryKey, skipToken, useMutation, useQuery as useConnectQuery } from '@connectrpc/connect-query';
import { useQueryClient } from '@tanstack/react-query'
import {
  Button,
  Card,
  CheckboxFormField,
  CardContent,
  CardHeader,
  CardTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Form,
  InputFormField,
  MultiSelectFormField,
  MultiSelectItem,
  SelectFormField,
  SelectItem,
  Separator,
  TextareaFormField,
} from '@ui/components'
import { ChevronDown } from 'lucide-react'
import { FunctionComponent, useEffect, useMemo } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { CurrencySelect } from '@/components/CurrencySelect'
import { Gate } from '@/components/Gate'
import { LocalId } from '@/components/LocalId'
import { Property } from '@/components/Property'
import { EntityActivityTimeline } from '@/features/activity/EntityActivityTimeline'
import {
  CREDIT_COUPON_HINT,
  couponDiscountLabel,
} from '@/features/productCatalog/coupons/discountLabel'
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
import { parseAndFormatDate, parseAndFormatDateOptional } from '@/utils/date'
import { useTypedParams } from '@/utils/params'

export const CouponDetails: FunctionComponent = () => {
  const queryClient = useQueryClient()

  const { couponLocalId } = useTypedParams<{ couponLocalId: string }>()

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
    onSuccess: invalidate,
    onError: error => toast.error(rpcErrorMessage(error, 'The coupon could not be removed')),
  })

  const updateCouponStatusMut = useMutation(updateCouponStatus, {
    onSuccess: invalidate,
    onError: error => toast.error(rpcErrorMessage(error, 'The coupon status could not be changed')),
  })

  const plansQuery = useConnectQuery(listPlans, {})

  const [, setTab] = useQueryState<string>('filter', '')

  const methods = useZodForm({
    schema: schemas.coupons.editComponentSchema,
    defaultValues: {
      description: '',
      discountType: 'percentage',
      percentage: undefined,
      planIds: [],
    },
  })

  const onSubmit = async (data: z.infer<typeof schemas.coupons.editComponentSchema>) => {
    editCouponMut.mutateAsync({
      couponId: query.data?.coupon?.id,
      description: data.description,
      discount: {
        discountType:
          data.discountType === 'fixed'
            ? {
                case: 'fixed' as const,
                value: {
                  amount: data.amount,
                  currency: data.currency,
                },
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
    })
  }

  const discountType = methods.watch('discountType')

  const coupon = query.data?.coupon
  // The server rejects switching a coupon between promotional credit and a discount.
  const isCreditCoupon = coupon?.discount?.discountType?.case === 'credit'

  // Reset form when coupon data loads or changes
  useEffect(() => {
    if (!coupon) return

    const kind = coupon.discount?.discountType
    if (kind?.case === 'credit') {
      methods.reset({
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

  const status = useMemo(() => {
    if (!coupon) {
      return 'Loading'
    }
    if (coupon.disabled) {
      return 'Disabled'
    }
    if (coupon.archivedAt) {
      return 'Archived'
    }
    if (
      coupon.redemptionCount &&
      coupon.redemptionLimit &&
      coupon.redemptionCount >= coupon.redemptionLimit
    ) {
      return 'Exhausted'
    }
    if (coupon.expiresAt && new Date(coupon.expiresAt).getTime() < Date.now()) {
      return 'Expired'
    }
    return 'Active'
  }, [coupon])

  if (!coupon) {
    return null
  }

  return (
    <div className="w-4/5">
      <Card className="min-h-[60%] flex flex-col">
        <CardHeader className="flex flex-row justify-between">
          <CardTitle className="content-center xl:space-x-2">
            <CopyToClipboardButton
              text={coupon.code}
              buttonVariant="ghost"
              buttonClassName="text-base font-semibold px-2"
            />
            <LocalId
              localId={coupon.localId}
              className="max-w-24 text-[10px]"
              buttonClassName="p-1 border-10"
            />
          </CardTitle>
          <div className="flex gap-0.5">
            <Gate perm="catalog:manage">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="primary"
                  size="sm"
                  // onClick={primaryAction.onClick}
                  hasIcon
                  className={' '}
                >
                  Actions <ChevronDown className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {coupon.archivedAt !== undefined ? (
                  <>
                    <DropdownMenuItem
                      key="enable"
                      onClick={() =>
                        updateCouponStatusMut
                          .mutateAsync({
                            action: CouponAction.ENABLE,
                            couponId: coupon.id,
                          })
                          .then(() => setTab('active'))
                      }
                    >
                      Restore & enable
                    </DropdownMenuItem>
                  </>
                ) : coupon.disabled ? (
                  <DropdownMenuItem
                    key="enable"
                    onClick={() =>
                      updateCouponStatusMut
                        .mutateAsync({
                          action: CouponAction.ENABLE,
                          couponId: coupon.id,
                        })
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
                        .mutateAsync({
                          action: CouponAction.DISABLE,
                          couponId: coupon.id,
                        })
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
                        .mutateAsync({
                          action: CouponAction.ARCHIVE,
                          couponId: coupon.id,
                        })
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
          </div>
        </CardHeader>
        <CardContent className="space-y-4 flex flex-col flex-1">
          <div className="grid grid-cols-1 xl:grid-cols-2 gap-y-2 gap-x-2">
            <Property label="Status" value={status} />
            <Property
              label="Expires"
              value={coupon.expiresAt ? parseAndFormatDate(coupon.expiresAt) : 'Never'}
            />
            <Property
              label="Redeemed"
              value={`${coupon.redemptionCount} / ${coupon.redemptionLimit ?? '∞'}`}
            />
            <Property
              label="Last usage"
              value={parseAndFormatDateOptional(coupon.lastRedemptionAt)}
            />
            {isCreditCoupon ? (
              <Property label="Discount" value={couponDiscountLabel(coupon.discount)} />
            ) : (
              <Property
                label="Recurring limit"
                value={
                  coupon.recurringValue
                    ? `${coupon.recurringValue} billing cycle(s)`
                    : 'Unlimited'
                }
              />
            )}
            {isCreditCoupon ? (
              <Property label="Per customer" value="Once per customer" />
            ) : (
              <Property label="Reusable" value={coupon.reusable ? 'Yes' : 'No'} />
            )}
            <Property
              label="Plan restrictions"
              value={
                coupon.planIds && coupon.planIds.length > 0
                  ? `${coupon.planIds.length} plan(s)`
                  : 'All plans'
              }
            />
          </div>
          <Separator />
          <Form {...methods}>
            <form
              className="h-full flex flex-col flex-1 justify-between"
              onSubmit={methods.handleSubmit(onSubmit)}
            >
              <div className="space-y-4">
                <TextareaFormField
                  name="description"
                  label="Description"
                  layout="horizontal"
                  control={methods.control}
                  placeholder="YC deal: 30% off for the first 6 months"
                />

                {isCreditCoupon ? (
                  <>
                    <div className="grid gap-2 md:grid-cols-12 items-center">
                      <span className="col-span-4 text-sm text-muted-foreground">Discount type</span>
                      <span className="col-span-8 text-sm">Promotional credit</span>
                    </div>
                    <p className="text-xs text-muted-foreground">{CREDIT_COUPON_HINT}</p>
                  </>
                ) : (
                  <SelectFormField
                    name="discountType"
                    label="Discount type"
                    layout="horizontal"
                    placeholder="Select..."
                    className="max-w-[320px]  "
                    control={methods.control}
                  >
                    <SelectItem value="fixed" key="fixed">
                      Fixed amount
                    </SelectItem>
                    <SelectItem value="percentage" key="percentage">
                      Percentage
                    </SelectItem>
                  </SelectFormField>
                )}

                {discountType === 'percentage' && (
                  <InputFormField
                    name="percentage"
                    label="Percentage"
                    required
                    layout="horizontal"
                    control={methods.control}
                    type="number"
                    placeholder="0"
                    rightText="%"
                  />
                )}

                {discountType === 'fixed' && (
                  <>
                    <InputFormField
                      name="amount"
                      required
                      label="Amount"
                      layout="horizontal"
                      control={methods.control}
                      type="number"
                      placeholder="Amount"
                    />
                    <CurrencySelect
                      name="currency"
                      label="Currency"
                      required
                      layout="horizontal"
                      control={methods.control}
                      placeholder="Currency"
                    />
                  </>
                )}

                {discountType === 'credit' && (
                  <>
                    <InputFormField
                      name="creditAmount"
                      required
                      label="Amount"
                      layout="horizontal"
                      control={methods.control}
                      type="text"
                      inputMode="decimal"
                      placeholder="0.00"
                    />
                    <CurrencySelect
                      name="currency"
                      label="Currency"
                      required
                      layout="horizontal"
                      control={methods.control}
                      placeholder="Currency"
                    />
                    <InputFormField
                      name="creditExpiresInDays"
                      label="Credit expires after (days)"
                      layout="horizontal"
                      control={methods.control}
                      type="number"
                      min={1}
                      max={3660}
                      placeholder="Never"
                      inputMode="numeric"
                    />
                    <CheckboxFormField
                      name="creditSubscriptionOnly"
                      label="Only the subscription it is redeemed on"
                      control={methods.control}
                      layout="horizontal"
                      description="The credit ends with that subscription."
                    />
                  </>
                )}

                <MultiSelectFormField
                  name="planIds"
                  label="Restrict to plans"
                  layout="horizontal"
                  control={methods.control}
                  placeholder="All plans (no restriction)"
                  hasSearch
                >
                  {plansQuery.data?.plans?.map(plan => (
                    <MultiSelectItem key={plan.id} value={plan.id}>
                      {plan.name}
                    </MultiSelectItem>
                  ))}
                </MultiSelectFormField>
              </div>
              <div className="flex justify-end">
                <Button
                  type="submit"
                  variant="primary"
                >
                  Save
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Activity</CardTitle>
        </CardHeader>
        <CardContent>
          <EntityActivityTimeline
            entityType="coupon"
            entityId={coupon.id}
            emptyLabel="No activity yet for this coupon"
          />
        </CardContent>
      </Card>
    </div>
  )
}
