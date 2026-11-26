import { CreditPaidFeatures } from '@/features/entitlements/customer/CreditPaidFeatures'
import { EffectiveEntitlementsCard } from '@/features/entitlements/customer/EffectiveEntitlementsCard'
import { OverridesCard } from '@/features/entitlements/customer/OverridesCard'
import { useQuery } from '@/lib/connectrpc'
import {
  getEffectiveEntitlements,
  listCreditPaidFeatures,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'

// Two columns only at 2xl: the tab sits in the customer page's 2/3 column, too narrow below that.
export const CustomerEntitlementsTab = ({
  customerId,
  customerName,
  onViewCredit,
}: {
  customerId: string
  customerName: string
  onViewCredit?: () => void
}) => {
  const paid = useQuery(listCreditPaidFeatures, { customerId }).data?.features ?? []
  const entitlements = useQuery(getEffectiveEntitlements, { customerId }).data?.entitlements ?? []
  const creditPaid = new Set(paid.map(p => p.featureId))

  return (
    <div className="grid items-start gap-6 2xl:grid-cols-[minmax(0,1fr)_320px]">
      <div className="flex min-w-0 flex-col gap-6">
        <EffectiveEntitlementsCard
          customerId={customerId}
          customerName={customerName}
          creditPaid={creditPaid}
        />
        {onViewCredit && paid.length > 0 && (
          <CreditPaidFeatures
            customerId={customerId}
            entitlements={entitlements}
            paid={paid}
            onViewCredit={onViewCredit}
          />
        )}
      </div>
      <OverridesCard customerId={customerId} />
    </div>
  )
}
