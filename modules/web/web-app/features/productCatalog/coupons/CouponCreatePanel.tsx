import { createConnectQueryKey, useMutation, useQuery } from '@connectrpc/connect-query';
import { useQueryClient } from '@tanstack/react-query'
import { InputFormField, MultiSelectFormField, MultiSelectItem } from '@ui/components'
import { customAlphabet } from 'nanoid'
import { FunctionComponent, useEffect, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router'
import { toast } from 'sonner'

import { emptyEligibility, fromDraft, isOpen } from '@/features/eligibility/eligibility'
import {
  CouponDiscountField,
  CouponDurationField,
  MoreConditions,
} from '@/features/productCatalog/coupons/CouponTermsFields'
import { couponDate, fixedDiscountOf } from '@/features/productCatalog/coupons/couponTerms'
import { CatalogEditPanel } from '@/features/productCatalog/generic/CatalogEditPanel'
import { useZodForm } from '@/hooks/useZodForm'
import { rpcErrorMessage } from '@/lib/rpcError'
import { schemas } from '@/lib/schemas'
import { createCoupon, listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { CouponDiscount_CreditScope } from '@/rpc/api/coupons/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

import type { CouponTermsFormFields } from '@/features/productCatalog/coupons/CouponTermsFields'
import type { Control } from 'react-hook-form'

const nanoid = customAlphabet('23456789ABCDEFGHJKLMNPQRSTUVWXYZ')

// An app path only: `//host` (or `/\host`) would leave the app.
const appPath = (value: string | null) =>
  value && /^\/(?![/\\])/.test(value) ? value : undefined

/**
 * `?planId=` prefills the plans; `?returnTo=` (an app path) is where the panel goes once saved
 * or closed, e.g. back to the plan that opened it.
 */
export const CouponCreatePanel: FunctionComponent = () => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const planId = searchParams.get('planId')
  const returnTo = appPath(searchParams.get('returnTo'))

  const createCouponMut = useMutation(createCoupon, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listCoupons.parent,
        cardinality: undefined
      }) })
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The coupon could not be created')),
  })

  const plansQuery = useQuery(listPlans, { pagination: { page: 0, perPage: 100 } })
  const [eligibility, setEligibility] = useState(emptyEligibility)

  const methods = useZodForm({
    schema: schemas.coupons.createCouponSchema,
    defaultValues: {
      code: `${nanoid(2)}-${nanoid(7)}`,
      description: '',
      discountType: 'percentage',
      percentage: undefined,
      redemptionLimit: undefined,
      duration: 'forever',
      durationMonths: undefined,
      reusable: false,
      planIds: planId ? [planId] : [],
      addOnIds: [],
      otherAmounts: [],
    },
  })
  const termsControl = methods.control as unknown as Control<CouponTermsFormFields>

  const discountType = methods.watch('discountType')

  // A credit coupon is redeemed once per customer: the hidden limits must not linger.
  useEffect(() => {
    if (discountType === 'credit') {
      methods.setValue('duration', 'forever')
      methods.setValue('durationMonths', undefined)
      methods.setValue('reusable', false)
    }
  }, [discountType])

  return (
    <CatalogEditPanel
      visible={true}
      closePanel={() => navigate(returnTo ?? '..')}
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
                      value: fixedDiscountOf(a.amount ?? '', a.currency, a.otherAmounts),
                    }
                  : a.discountType === 'credit'
                    ? {
                        case: 'credit' as const,
                        value: {
                          amount: a.creditAmount,
                          currency: a.currency,
                          expiresInDays: a.creditExpiresInDays,
                          scope: a.creditSubscriptionOnly
                            ? CouponDiscount_CreditScope.SUBSCRIPTION
                            : CouponDiscount_CreditScope.CUSTOMER,
                        },
                      }
                    : {
                        case: 'percentage' as const,
                        value: { percentage: a.percentage },
                      },
            },
            validFrom: couponDate(a.validFrom),
            expiresAt: couponDate(a.expiresAt),
            // Once is the first invoice; months count from the first paid period.
            recurringValue: a.discountType !== 'credit' && a.duration === 'once' ? 1 : undefined,
            durationMonths:
              a.discountType !== 'credit' && a.duration === 'months' ? a.durationMonths : undefined,
            redemptionLimit: a.redemptionLimit,
            reusable: a.discountType === 'credit' ? false : a.reusable,
            planIds: a.planIds ?? [],
            addOnIds: a.addOnIds ?? [],
            eligibility: isOpen(fromDraft(eligibility)) ? undefined : fromDraft(eligibility),
          })
          .then(() => void 0)
      }
    >
      <div className="mt-4 space-y-5 pb-4">
        <div className="grid grid-cols-[2fr_3fr] gap-3">
          <InputFormField
            name="code"
            label="Code"
            required
            control={methods.control}
            type="text"
            placeholder="Coupon code"
          />
          <InputFormField
            name="description"
            label="Description"
            control={methods.control}
            type="text"
            placeholder="YC deal: 30% off for the first 6 months"
          />
        </div>

        <CouponDiscountField control={termsControl} />

        {discountType !== 'credit' && <CouponDurationField control={termsControl} withOnce />}

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
          control={termsControl}
          kind={discountType}
          creating
          eligibility={eligibility}
          onEligibilityChange={setEligibility}
        />
      </div>
    </CatalogEditPanel>
  )
}
