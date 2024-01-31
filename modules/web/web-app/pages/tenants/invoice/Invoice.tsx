import { spaces } from '@md/foundation'
import { Flex, Skeleton } from '@ui/components'
import { ChevronLeftIcon } from 'lucide-react'
import { Fragment } from 'react'
import { useNavigate } from 'react-router-dom'

import { TenantPageLayout } from '@/components/layouts'
import { StatusPill } from '@/features/invoices/StatusPill'
import { CustomerCard } from '@/features/invoices/cards/CustomerCard'
import { InvoiceCard } from '@/features/invoices/cards/InvoiceCard'
import { SubscriptionCard } from '@/features/invoices/cards/SubscriptionCard'
import { useQuery } from '@/lib/connectrpc'
import { getInvoice } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { useTypedParams } from '@/utils/params'

export const Invoice = () => {
  const navigate = useNavigate()
  const { invoiceId } = useTypedParams<{ invoiceId: string }>()
  const invoiceQuery = useQuery(
    getInvoice,
    {
      id: invoiceId ?? '',
    },
    { enabled: Boolean(invoiceId) }
  )

  const data = invoiceQuery.data?.invoice
  const isLoading = invoiceQuery.isLoading

  return (
    <Fragment>
      <TenantPageLayout title="Invoices">
        <Flex direction="column" gap={spaces.space9} fullHeight>
          {isLoading || !data ? (
            <>
              <Skeleton height={16} width={50} />
              <Skeleton height={44} />
            </>
          ) : (
            <>
              <div className="flex justify-between">
                <div className="flex gap-2 items-center text-2xl">
                  <ChevronLeftIcon
                    className="font-semibold cursor-pointer"
                    onClick={() => navigate('..')}
                  />
                  <h2 className="font-semibold">{data.id}</h2>
                </div>
                <div className="text-sm">
                  <StatusPill status={data.status} />
                </div>
              </div>
              <div className="flex h-full gap-4">
                <div className="flex flex-col gap-2 border-r-2 border-slate-600 pr-4">
                  <div className="text-4xl font-semibold">$ to be computed</div>
                </div>
                <div className="flex-1 flex flex-col gap-2">
                  <InvoiceCard invoice={data} />
                  <CustomerCard invoice={data} />
                  <SubscriptionCard invoice={data} />
                </div>
              </div>
            </>
          )}
        </Flex>
      </TenantPageLayout>
    </Fragment>
  )
}
