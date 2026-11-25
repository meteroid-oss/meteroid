import { skipToken } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'

import type { Coupon } from '@/rpc/api/coupons/v1/models_pb'

/** The coupon worth naming for a plan: not single-use, then the most redeemed. */
export const featuredCoupon = (coupons: Coupon[]): Coupon | undefined =>
  coupons
    .filter(coupon => coupon.redemptionLimit !== 1)
    .reduce<Coupon | undefined>(
      (best, coupon) => (!best || coupon.redemptionCount > best.redemptionCount ? coupon : best),
      undefined
    )

/**
 * A plan's live coupons, most redeemed first, one page of `perPage`: a plan can have thousands of
 * single-use codes, so `total` comes from the page's count, never from the list.
 */
export const usePlanCoupons = (planId: string | undefined, perPage: number) => {
  const data = useQuery(
    listCoupons,
    planId
      ? {
          planId,
          filter: ListCouponRequest_CouponFilter.ACTIVE,
          orderBy: 'redemption_count.desc',
          pagination: { page: 0, perPage },
        }
      : skipToken
  ).data
  const coupons = data?.coupons ?? []
  const total = Math.max(Number(data?.paginationMeta?.totalItems ?? 0), coupons.length)
  return { coupons, total, featured: featuredCoupon(coupons) }
}
