import { useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import {
  Button,
  Calendar,
  FormControl,
  GenericFormField,
  InputFormField,
  Popover,
  PopoverContent,
  PopoverTrigger,
  SelectFormField,
  SelectItem,
  TextareaFormField,
} from '@ui/components'
import { cn } from '@ui/lib'
import { format } from 'date-fns'
import { CalendarIcon } from 'lucide-react'
import { customAlphabet } from 'nanoid'
import { FunctionComponent } from 'react'
import { useNavigate } from 'react-router-dom'

import { CurrencySelect } from '@/components/CurrencySelect'
import { CatalogEditPanel } from '@/features/productCatalog/generic/CatalogEditPanel'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import { createCoupon, listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'

const nanoid = customAlphabet('23456789ABCDEFGHJKLMNPQRSTUVWXYZ')
export const CouponCreatePanel: FunctionComponent = () => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const createCouponMut = useMutation(createCoupon, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listCoupons.service.typeName] })
    },
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
          })
          .then(() => void 0)
      }
    >
      <div>
        <section className="space-y-4">
          <div className="space-y-6 py-2">
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
                        {field.value ? format(field.value, 'PPP') : <span>Pick a date</span>}
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
                    {/* // TODO time picker + timezone */}
                  </PopoverContent>
                </Popover>
              )}
            />

            <InputFormField
              name="redemptionLimit"
              label="Redemption limit"
              layout="horizontal"
              control={methods.control}
              type="number"
              placeholder="Unlimited"
              inputMode="numeric"
            />

            {/* 
TODO change to duration_months
       
recurringValue
reusable
*/}
          </div>
        </section>
      </div>
    </CatalogEditPanel>
  )
}
