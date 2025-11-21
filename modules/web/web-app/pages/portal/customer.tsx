import { Card, Skeleton } from '@md/ui'
import { AlertCircle } from 'lucide-react'
import { useState } from 'react'

import { BillingInfo } from '@/features/checkout/components/BillingInfo'
import { useQuery } from '@/lib/connectrpc'
import { getCustomerPortalOverview } from '@/rpc/portal/customer/v1/customer-PortalCustomerService_connectquery'
import { useForceTheme } from 'providers/ThemeProvider'

import { CustomerPortalInvoices } from './customer/CustomerPortalInvoices'
import { CustomerPortalPaymentMethods } from './customer/CustomerPortalPaymentMethods'
import { CustomerPortalSubscriptions } from './customer/CustomerPortalSubscriptions'

export const PortalCustomer = () => {
  useForceTheme('light')
  const [isAddressEditing, setIsAddressEditing] = useState(false)

  const overviewQuery = useQuery(getCustomerPortalOverview)
  const { data, error, isLoading, refetch } = overviewQuery

  if (error) {
    return (
      <div className="min-h-screen w-full bg-[#00000002] flex items-center justify-center">
        <div className="max-w-md mx-auto px-6 py-12 text-center">
          <AlertCircle className="h-8 w-8 text-muted-foreground mb-4 mx-auto" />
          <h2 className="text-md font-semibold text-gray-800 mb-2">Something went wrong</h2>
          <p className="text-gray-800 text-sm">
            There may be a connection issue or your session might be expired.
          </p>
        </div>
      </div>
    )
  }

  if (isLoading || !data?.overview) {
    return (
      <div className="min-h-screen w-full bg-[#00000002]">
        <div className="container max-w-6xl mx-auto py-12 px-4">
          <Skeleton height={32} width={200} className="mb-8" />
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
            <Skeleton height={400} />
            <Skeleton height={400} />
          </div>
        </div>
      </div>
    )
  }

  const { customer, activeSubscriptions, paymentMethods, cardConnectionId, directDebitConnectionId } =
    data.overview

  if (!customer) {
    return null
  }

  return (
    <div className="min-h-screen w-full bg-[#00000002]">
      <div className="container max-w-6xl mx-auto py-12 px-4 md:px-6 h-full">
        {/* Header */}
        <div className="mb-8">
          <h1 className="text-2xl md:text-3xl font-semibold text-gray-900">Customer Portal</h1>
          <p className="text-sm text-gray-600 mt-1">
            Manage your subscriptions, invoices, and billing information
          </p>
        </div>

        {/* Main Content Grid */}
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 h-full overflow-y-auto">
          {/* Left Column - Billing & Payment */}
          <div className="space-y-6">
            {/* Billing Information */}
            <BillingInfo
              customer={customer}
              isEditing={isAddressEditing}
              setIsEditing={setIsAddressEditing}
            />

            {/* Payment Methods */}
            <CustomerPortalPaymentMethods
              paymentMethods={paymentMethods || []}
              cardConnectionId={cardConnectionId}
              directDebitConnectionId={directDebitConnectionId}
              onRefetch={() => refetch()}
            />
          </div>

          {/* Right Column - Subscriptions & Invoices */}
          <div className="space-y-6">
            {/* Subscriptions */}
            <Card className="border-0 shadow-sm">
              <div className="p-6">
                <h2 className="text-md font-medium mb-4">Subscriptions</h2>
                <CustomerPortalSubscriptions subscriptions={activeSubscriptions || []} />
              </div>
            </Card>

            {/* Recent Invoices */}
            <Card className="border-0 shadow-sm">
              <div className="p-6">
                <h2 className="text-md font-medium mb-4">Invoices</h2>
                <CustomerPortalInvoices />
              </div>
            </Card>
          </div>
        </div>
      </div>
    </div>
  )
}
