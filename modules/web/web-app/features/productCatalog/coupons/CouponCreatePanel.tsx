import { useMutation, useQuery } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import {
  Button,
  Calendar,
  CheckboxFormField,
  FormControl,
  GenericFormField,
  InputFormField,
  MultiSelectFormField,
  MultiSelectItem,
  Popover,
  PopoverContent,
  PopoverTrigger,
  SelectFormField,
  SelectItem,
  TextareaFormField,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@ui/components'
import { cn } from '@ui/lib'
import { format } from 'date-fns'
import { CalendarIcon, InfoIcon } from 'lucide-react'
import { customAlphabet } from 'nanoid'
import { FunctionComponent } from 'react'
import { useNavigate } from 'react-router-dom'

import { CurrencySelect } from '@/components/CurrencySelect'
import { CatalogEditPanel } from '@/features/productCatalog/generic/CatalogEditPanel'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { createCoupon, listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'

const nanoid = customAlphabet('23456789ABCDEFGHJKLMNPQRSTUVWXYZ')

const InfoTooltip = ({ children }: { children: React.ReactNode }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <InfoIcon className="h-3.5 w-3.5 text-muted-foreground cursor-help" />
    </TooltipTrigger>
    <TooltipContent className="max-w-72">{children}</TooltipContent>
  </Tooltip>
)

export const CouponCreatePanel: FunctionComponent = () => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const createCouponMut = useMutation(createCoupon, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listCoupons.service.typeName] })
    },
  })

  const plansQuery = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const methods = useZodForm({
    schema: schemas.coupons.createCouponSchema,
    defaultValues: {
      code: `${nanoid(2)}-${nanoid(7)}`,
      description: '',
      discountType: 'percentage',
      percentage: undefined,
      redemptionLimit: undefined,
      recurringValue: undefined,
      reusable: false,
      planIds: [],
    },
  })

  const discountType = methods.watch('discountType')

  return (
    <CatalogEditPanel
      visible={true}
      closePanel={() => navigate('..')}
      title="Create coupon"
      methods={methods}
      onSubmit={a =>
        createCouponMut
          .mutateAsync({
            code: a.code,
            description: a.description,
            discount: {
              discountType:
                a.discountType === 'fixed'
                  ? {
                      case: 'fixed' as const,
                      value: {
                        amount: a.amount,
                        currency: a.currency,
                      },
                    }
                  : {
                      case: 'percentage' as const,
                      value: { percentage: a.percentage },
                    },
            },
            expiresAt: a.expiresAt ? format(a.expiresAt, "y-MM-dd'T'HH:mm:ss") : undefined, // TODO time & timezone, to UTC
            recurringValue: a.recurringValue,
            redemptionLimit: a.redemptionLimit,
            reusable: a.reusable,
            planIds: a.planIds ?? [],
          })
          .then(() => void 0)
      }
    >
      <TooltipProvider delayDuration={100}>
      <div className="space-y-6">
        {/* Basic Info */}
        <section className="space-y-4 mt-4">
          <div className="space-y-4">
            <InputFormField
              name="code"
              label="Code"
              layout="horizontal"
              required
              control={methods.control}
              type="text"
              placeholder="Coupon code"
            />
            <TextareaFormField
              name="description"
              label="Description"
              layout="horizontal"
              control={methods.control}
              placeholder="YC deal: 30% off for the first 6 months"
            />
          </div>
        </section>

        <hr className="border-border" />

        {/* Discount */}
        <section className="space-y-4">
          <h3 className="text-sm font-medium text-muted-foreground">Discount</h3>
          <div className="space-y-4">
            <SelectFormField
              name="discountType"
              label="Type"
              layout="horizontal"
              placeholder="Select..."
              className="max-w-[320px]"
              control={methods.control}
            >
              <SelectItem value="percentage" key="percentage">
                Percentage
              </SelectItem>
              <SelectItem value="fixed" key="fixed">
                Fixed amount
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
          </div>
        </section>

        <hr className="border-border" />

        {/* Limits */}
        <section className="space-y-4">
          <h3 className="text-sm font-medium text-muted-foreground">Limits</h3>
          <div className="space-y-4">
            <GenericFormField
              control={methods.control}
              layout="horizontal"
              name="expiresAt"
              label="Expiration"
              render={({ field }) => (
                <Popover>
                  <PopoverTrigger asChild>
                    <FormControl>
                      <Button
                        variant="outline"
                        className={cn(
                          'w-full pl-3 text-left font-normal col-span-8',
                          !field.value && 'text-muted-foreground'
                        )}
                      >
                        {field.value ? format(field.value, 'PPP') : <span>No expiration</span>}
                        <CalendarIcon className="ml-auto h-4 w-4 opacity-50" />
                      </Button>
                    </FormControl>
                  </PopoverTrigger>
                  <PopoverContent className="w-auto p-0" align="start">
                    <Calendar
                      mode="single"
                      selected={field.value}
                      onSelect={field.onChange}
                      disabled={date => date < new Date()}
                      initialFocus
                    />
                  </PopoverContent>
                </Popover>
              )}
            />

            <InputFormField
              name="redemptionLimit"
              label="Max redemptions"
              layout="horizontal"
              control={methods.control}
              type="number"
              placeholder="Unlimited"
              inputMode="numeric"
              labelTooltip={
                <InfoTooltip>
                  Total number of times this coupon can be used across all customers
                </InfoTooltip>
              }
            />

            <InputFormField
              name="recurringValue"
              label="Billing cycles"
              layout="horizontal"
              control={methods.control}
              type="number"
              placeholder="Forever"
              inputMode="numeric"
              labelTooltip={
                <InfoTooltip>
                  Number of billing cycles the discount applies. After this, the subscription
                  continues at full price.
                </InfoTooltip>
              }
            />
          </div>
        </section>

        <hr className="border-border" />

        {/* Restrictions */}
        <section className="space-y-4 pb-4">
          <h3 className="text-sm font-medium text-muted-foreground">Restrictions</h3>
          <div className="space-y-4">
            <CheckboxFormField
              name="reusable"
              label="Reusable"
              control={methods.control}
              layout="horizontal"
              description="Same customer can use on multiple subscriptions"
            />

            <MultiSelectFormField
              name="planIds"
              label="Plans"
              layout="horizontal"
              control={methods.control}
              placeholder="All plans"
              hasSearch
              labelTooltip={
                <InfoTooltip>
                  Restrict this coupon to specific plans. Leave empty to allow on all plans.
                </InfoTooltip>
              }
            >
              {plansQuery.data?.plans?.map(plan => (
                <MultiSelectItem key={plan.id} value={plan.id}>
                  {plan.name}
                </MultiSelectItem>
              ))}
            </MultiSelectFormField>
          </div>
        </section>
      </div>
      </TooltipProvider>
    </CatalogEditPanel>
  )
}
