import { Skeleton } from '@md/ui'
import { AlertCircle } from 'lucide-react'

import InvoicePaymentFlow from '@/features/invoice-payment/InvoicePaymentFlow'
import { useQuery } from '@/lib/connectrpc'
import { getInvoicePayment } from '@/rpc/portal/invoice/v1/invoice-PortalInvoiceService_connectquery'
import { useTypedParams } from '@/utils/params'
import { useForceTheme } from 'providers/ThemeProvider'

export const PortalInvoicePayment = () => {
  useForceTheme('light')

  const invoiceId = useTypedParams<{ invoiceId: string }>().invoiceId

  const invoicePaymentQuery = useQuery(getInvoicePayment, {
    invoiceId,
  })

  const data = invoicePaymentQuery.data?.invoice
  const error = invoicePaymentQuery.error
  const isLoading = invoicePaymentQuery.isLoading

  if (error) {
    return (
      <div className="h-full w-full bg-[#00000002]">
        <div className="flex flex-col items-center justify-center h-full max-w-md mx-auto px-6 py-12 text-center">
          <AlertCircle className="h-8 w-8 text-muted-foreground mb-4" />
          <h2 className="text-md font-semibold text-gray-800 mb-2">Something went wrong</h2>
          <p className="text-gray-800 text-sm">
            There may be a connection issue, your session might be expired or completed, or our
            payment system is temporarily unavailable
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full bg-[#00000002]">
      <div className="flex flex-col gap-4 h-full">
        {isLoading || !data ? (
          <>
            <Skeleton height={16} width={50} />
            <Skeleton height={44} />
          </>
        ) : (
          <InvoicePaymentFlow invoicePaymentData={data} />
        )}
      </div>
    </div>
  )
}
