import { createConnectQueryKey, skipToken, useMutation, useQuery as useConnectQuery } from '@connectrpc/connect-query';
import { useQueryClient } from '@tanstack/react-query'
import {
  Badge,
  Button,
  CheckboxFormField,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Form,
  InputFormField,
  Modal,
  MultiSelectFormField,
  MultiSelectItem,
  SelectFormField,
  SelectItem,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  TextareaFormField,
} from '@ui/components'
import { ChevronDown } from 'lucide-react'
import { FunctionComponent, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import ConfirmationModal from '@/components/ConfirmationModal'
import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { CurrencySelect } from '@/components/CurrencySelect'
import { Gate } from '@/components/Gate'
import { LocalId } from '@/components/LocalId'
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

/**
 * A coupon, in the same side panel as creating one: its code and handle with the actions, its
 * properties, the editable fields, and its activity. Closing returns to the list.
 */
export const CouponDetails: FunctionComponent = () => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [confirmingClose, setConfirmingClose] = useState(false)

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
    // The coupon is gone: back to the list.
    onSuccess: async () => {
      await invalidate()
      navigate('..')
    },
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

  // Unsaved edits ask first, as the create panel does.
  const close = () => {
    if (methods.formState.isDirty) setConfirmingClose(true)
    else navigate('..')
  }
  const isDirty = methods.formState.isDirty

  return (
    <>
      <Sheet open onOpenChange={open => !open && close()}>
        <SheetContent size="small" className="flex flex-col gap-0 p-0">
          {coupon && (
            <Form {...methods}>
              <form className="flex min-h-full flex-col" onSubmit={methods.handleSubmit(onSubmit)}>
                {/* Who it is: the code (copyable) and its state, what it gives, its handle. */}
                <SheetHeader className="gap-2 border-b-[0.5px] border-foreground/[0.102] px-6 pb-4 pt-6 pr-14">
                  <div className="flex items-center justify-between gap-3">
                    <SheetTitle className="flex min-w-0 items-center gap-2">
                      <CopyToClipboardButton
                        text={coupon.code}
                        buttonVariant="ghost"
                        buttonClassName="-ml-2 h-8 px-2 font-mono text-base font-semibold"
                      />
                      <Badge variant={STATUS_VARIANTS[status] ?? 'secondary'} size="sm">
                        {status}
                      </Badge>
                    </SheetTitle>
                    <Gate perm="catalog:manage">
                      <CouponActions
                        archived={coupon.archivedAt !== undefined}
                        disabled={coupon.disabled}
                        deletable={!coupon.lastRedemptionAt}
                        onAction={(action, then) =>
                          updateCouponStatusMut
                            .mutateAsync({ action, couponId: coupon.id })
                            .then(() => setTab(then))
                        }
                        onDelete={() => removeCouponMut.mutateAsync({ couponId: coupon.id })}
                      />
                    </Gate>
                  </div>
                  <div className="flex items-center gap-2 text-[13px] text-muted-foreground">
                    <span className="truncate">{couponDiscountLabel(coupon.discount)}</span>
                    <span aria-hidden>·</span>
                    <LocalId localId={coupon.localId} className="max-w-28 text-[10px]" />
                  </div>
                </SheetHeader>

                <div className="flex flex-1 flex-col gap-6 px-6 py-5">
                  {/* The create panel's sections, in its order: the same place for the same thing. */}
                  <PanelSection title="General">
                    <TextareaFormField
                      name="description"
                      label="Description"
                      layout="horizontal"
                      control={methods.control}
                      placeholder="YC deal: 30% off for the first 6 months"
                    />
                  </PanelSection>

                  <PanelSection title="Discount">
                    {isCreditCoupon ? (
                      <ReadRow label="Type" hint={CREDIT_COUPON_HINT}>
                        Promotional credit
                      </ReadRow>
                    ) : (
                      <SelectFormField
                        name="discountType"
                        label="Type"
                        layout="horizontal"
                        placeholder="Select…"
                        control={methods.control}
                      >
                        <SelectItem value="percentage">Percentage</SelectItem>
                        <SelectItem value="fixed">Fixed amount</SelectItem>
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

                    {(discountType === 'fixed' || discountType === 'credit') && (
                      <>
                        <InputFormField
                          key={discountType}
                          name={discountType === 'credit' ? 'creditAmount' : 'amount'}
                          required
                          label="Amount"
                          layout="horizontal"
                          control={methods.control}
                          type={discountType === 'credit' ? 'text' : 'number'}
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
                      </>
                    )}

                    {discountType === 'credit' && (
                      <>
                        <InputFormField
                          name="creditExpiresInDays"
                          label="Credit expires after"
                          layout="horizontal"
                          control={methods.control}
                          type="number"
                          min={1}
                          max={3660}
                          placeholder="Never"
                          inputMode="numeric"
                          rightText="days"
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
                  </PanelSection>

                  <PanelSection title="Limits">
                    <ReadRow label="Expiration">
                      {coupon.expiresAt ? parseAndFormatDate(coupon.expiresAt) : 'Never'}
                    </ReadRow>
                    <ReadRow label="Redemptions">
                      <Redemptions
                        count={coupon.redemptionCount}
                        limit={coupon.redemptionLimit}
                      />
                    </ReadRow>
                    <ReadRow label="Last used">
                      {parseAndFormatDateOptional(coupon.lastRedemptionAt) || 'Never'}
                    </ReadRow>
                    {!isCreditCoupon && (
                      <ReadRow label="Billing cycles">
                        {coupon.recurringValue ? `${coupon.recurringValue}` : 'Forever'}
                      </ReadRow>
                    )}
                  </PanelSection>

                  <PanelSection title="Restrictions">
                    <ReadRow label="Per customer">
                      {isCreditCoupon
                        ? 'Once per customer'
                        : coupon.reusable
                          ? 'Reusable on several subscriptions'
                          : 'Once per customer'}
                    </ReadRow>
                    <MultiSelectFormField
                      name="planIds"
                      label="Plans"
                      layout="horizontal"
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
                  </PanelSection>

                  <PanelSection title="Activity">
                    <EntityActivityTimeline
                      entityType="coupon"
                      entityId={coupon.id}
                      emptyLabel="No activity yet for this coupon"
                    />
                  </PanelSection>
                </div>

                {/* Saving stays in reach while the panel scrolls; it wakes up once something changed. */}
                <Gate perm="catalog:manage">
                  <SheetFooter className="sticky bottom-0 flex-row justify-end gap-2 border-t-[0.5px] border-foreground/[0.102] bg-background px-6 py-3 dark:bg-[#171717]">
                    <Button
                      type="button"
                      variant="secondary"
                      disabled={!isDirty || editCouponMut.isPending}
                      onClick={() => methods.reset()}
                    >
                      Discard
                    </Button>
                    <Button type="submit" disabled={!isDirty || editCouponMut.isPending}>
                      {editCouponMut.isPending ? 'Saving…' : 'Save changes'}
                    </Button>
                  </SheetFooter>
                </Gate>
              </form>
            </Form>
          )}
        </SheetContent>
      </Sheet>
      <ConfirmationModal
        visible={confirmingClose}
        header="Confirm to close"
        buttonLabel="Confirm"
        onSelectCancel={() => setConfirmingClose(false)}
        onSelectConfirm={() => {
          setConfirmingClose(false)
          methods.reset()
          navigate('..')
        }}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-muted-foreground">
            There are unsaved changes. Are you sure you want to close the panel? Your changes will
            be lost.
          </p>
        </Modal.Content>
      </ConfirmationModal>
    </>
  )
}

const STATUS_VARIANTS: Record<string, 'success' | 'secondary' | 'warning'> = {
  Active: 'success',
  Disabled: 'secondary',
  Archived: 'secondary',
  Exhausted: 'warning',
  Expired: 'secondary',
}

/** A titled group of the panel, as the create panel's: a quiet heading over its rows. */
const PanelSection = ({ title, children }: { title: string; children: React.ReactNode }) => (
  <section className="flex flex-col gap-3 border-b-[0.5px] border-foreground/[0.102] pb-6 last:border-b-0 last:pb-0">
    <h3 className="text-sm font-medium text-muted-foreground">{title}</h3>
    <div className="flex flex-col gap-4">{children}</div>
  </section>
)

/** A value that can't be edited here, on the form's grid: label on the left, value on the right. */
const ReadRow = ({
  label,
  hint,
  children,
}: {
  label: string
  hint?: string
  children: React.ReactNode
}) => (
  <div className="grid items-start gap-2 md:grid-cols-12">
    <span className="pt-px text-xs text-muted-foreground md:col-span-4">{label}</span>
    <div className="flex flex-col gap-1 text-[13px] text-foreground md:col-span-8">
      {children}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  </div>
)

/** How often the coupon was used, with a bar against its limit when it has one. */
const Redemptions = ({ count, limit }: { count: number; limit?: number }) => (
  <div className="flex flex-col gap-1.5">
    <span className="tabular-nums">
      {count}
      <span className="text-muted-foreground"> / {limit ?? '∞'}</span>
    </span>
    {limit ? (
      <span className="h-1 w-full max-w-40 overflow-hidden rounded-full bg-foreground/[0.08]">
        <span
          className="block h-full rounded-full bg-foreground/60"
          style={{ width: `${Math.min(100, (count / limit) * 100)}%` }}
        />
      </span>
    ) : null}
  </div>
)

/** The coupon's status actions, by its state: restore, enable or disable, archive, delete. */
const CouponActions = ({
  archived,
  disabled,
  deletable,
  onAction,
  onDelete,
}: {
  archived: boolean
  disabled: boolean
  /** A coupon never redeemed can be deleted outright. */
  deletable: boolean
  /** Runs the action, then shows the list tab the coupon moved to. */
  onAction: (action: CouponAction, tab: string) => void
  onDelete: () => void
}) => (
  <DropdownMenu>
    <DropdownMenuTrigger asChild>
      <Button variant="secondary" size="sm" className="gap-1.5 pr-2">
        Actions
        <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" aria-hidden />
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end">
      {archived ? (
        <DropdownMenuItem onClick={() => onAction(CouponAction.ENABLE, 'active')}>
          Restore & enable
        </DropdownMenuItem>
      ) : disabled ? (
        <DropdownMenuItem onClick={() => onAction(CouponAction.ENABLE, 'active')}>
          Enable
        </DropdownMenuItem>
      ) : (
        <DropdownMenuItem onClick={() => onAction(CouponAction.DISABLE, 'inactive')}>
          Disable
        </DropdownMenuItem>
      )}
      {!archived && (
        <DropdownMenuItem onClick={() => onAction(CouponAction.ARCHIVE, 'archived')}>
          Archive
        </DropdownMenuItem>
      )}
      {deletable && (
        <DropdownMenuItem variant="destructive" onClick={onDelete}>
          Delete
        </DropdownMenuItem>
      )}
    </DropdownMenuContent>
  </DropdownMenu>
)
