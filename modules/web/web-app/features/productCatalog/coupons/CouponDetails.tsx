import { disableQuery, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import {
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Form,
  InputFormField,
  SelectFormField,
  SelectItem,
  Separator,
  TextareaFormField,
} from '@ui/components'
import { ChevronDown } from 'lucide-react'
import { FunctionComponent, useMemo } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import { LocalId } from '@/components/LocalId'
import { Property } from '@/components/Property'
import { useQueryState } from '@/hooks/useQueryState'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import {
  editCoupon,
  getCoupon,
  listCoupons,
  removeCoupon,
  updateCouponStatus,
} from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { CouponAction } from '@/rpc/api/coupons/v1/coupons_pb'
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
      : disableQuery
  )

  const invalidate = async () => {
    await queryClient.invalidateQueries({ queryKey: [listCoupons.service.typeName] })
  }

  const editCouponMut = useMutation(editCoupon, {
    onSuccess: async () => {
      await invalidate()
      toast.success('Coupon updated !')
    },
  })

  const removeCouponMut = useMutation(removeCoupon, {
    onSuccess: invalidate,
  })

  const updateCouponStatusMut = useMutation(updateCouponStatus, {
    onSuccess: invalidate,
  })

  const [, setTab] = useQueryState<string>('filter', '')

  const methods = useZodForm({
    schema: schemas.coupons.editComponentSchema,
    defaultValues: {
      description: '',
      discountType: 'percentage',
      percentage: '',
      amount: '',
      currency: '',
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
            : {
                case: 'percentage' as const,
                value: { percentage: data.percentage },
              },
      },
    })
  }

  const discountType = methods.watch('discountType')

  const coupon = query.data?.coupon

  const status = useMemo(() => {
    if (!coupon) {
      return 'Loading'
    }

    methods.reset({
      amount:
        coupon.discount?.discountType?.case === 'fixed'
          ? coupon.discount.discountType.value.amount
          : '',
      currency:
        coupon.discount?.discountType?.case === 'fixed'
          ? coupon.discount.discountType.value.currency
          : '',
      description: coupon.description,
      discountType: coupon.discount?.discountType?.case,
      percentage:
        coupon.discount?.discountType?.case === 'percentage'
          ? coupon.discount.discountType.value.percentage
          : '',
    })

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
          <CardTitle className="content-center  xl:space-x-2">
            <span>{coupon.code}</span>
            <LocalId
              localId={coupon.localId}
              className="max-w-24 text-[10px] "
              buttonClassName="p-1 border-10"
            />
          </CardTitle>
          <div className="flex gap-0.5">
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
                    Achive
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
          </div>
          <Separator />
          <Form {...methods}>
            <form
              className="h-full flex flex-col flex-1 justify-between"
              onSubmit={methods.handleSubmit(async values => {
                await onSubmit(values)
                methods.reset()
              })}
            >
              <div className="space-y-4">
                <TextareaFormField
                  name="description"
                  label="Description"
                  layout="horizontal"
                  control={methods.control}
                  placeholder="YC deal: 30% off for the first 6 months"
                />

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
                    asString
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
                      asString
                      type="number"
                      placeholder="Amount"
                    />
                    <InputFormField // TODO
                      name="currency"
                      label="Currency"
                      required
                      layout="horizontal"
                      control={methods.control}
                      type="text"
                      placeholder="Currency"
                    />
                  </>
                )}
              </div>
              <div className="flex justify-end">
                <Button
                  type="submit"
                  variant="primary"
                  disabled={!methods.formState.isValid || !methods.formState.isDirty}
                >
                  Save
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>
    </div>
  )
}
