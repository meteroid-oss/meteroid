import { Skeleton } from '@md/ui'
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
      <div className="min-h-screen bg-white">
        <div className="border-b border-gray-200">
          <div className="max-w-5xl mx-auto px-6 md:px-12 py-6">
            <Skeleton height={32} width={200} />
          </div>
        </div>
        <div className="max-w-5xl mx-auto px-6 md:px-12 py-8">
          <Skeleton height={16} width={100} className="mb-2" />
          <Skeleton height={180} className="mb-6 rounded-lg" />
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6 mb-6">
            <div>
              <Skeleton height={16} width={120} className="mb-2" />
              <Skeleton height={120} className="rounded-lg" />
            </div>
            <div>
              <Skeleton height={16} width={140} className="mb-2" />
              <Skeleton height={120} className="rounded-lg" />
            </div>
          </div>
          <Skeleton height={16} width={80} className="mb-2" />
          <Skeleton height={200} className="rounded-lg" />
        </div>
      </div>
    )
  }

  const {
    customer,
    activeSubscriptions,
    paymentMethods,
    cardConnectionId,
    directDebitConnectionId,
    invoicingEntityName,
    invoicingEntityLogoUrl,
    invoicingEntityBrandColor,
  } = data.overview

  if (!customer) {
    return null
  }

  return (
    <div className="min-h-screen bg-white">
      {/* Header */}
      <div className="border-b border-gray-200">
        <div className="max-w-5xl mx-auto px-6 md:px-12 py-6 flex items-center justify-between">
          <div className="flex items-center gap-4">
            {invoicingEntityLogoUrl && (
              <img
                src={invoicingEntityLogoUrl}
                alt={invoicingEntityName || 'Company logo'}
                className="h-8 w-auto object-contain"
              />
            )}
            <div>
              <p className="text-md font-medium text-gray-900">
                {invoicingEntityName || customer.name} • Billing portal
              </p>
              <p className="text-sm text-gray-600">{customer.billingEmail}</p>
            </div>
          </div>
        </div>
      </div>

      {/* Main Content */}
      <div className="max-w-5xl mx-auto px-6 md:px-12 py-8">
        {/* Subscription Section */}
        <div className="mb-6">
          <h2 className="text-xs font-medium text-gray-500 mb-2">Subscription</h2>
          <div className="bg-white border border-gray-200 rounded p-4">
            <CustomerPortalSubscriptions subscriptions={activeSubscriptions || []} />
          </div>
        </div>

        {/* Two Column Layout */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6 mb-6">
          {/* Payment Method */}
          <div>
            <h2 className="text-xs font-medium text-gray-500 mb-2">Payment method</h2>
            <div className="bg-white border border-gray-200 rounded p-4">
              <CustomerPortalPaymentMethods
                paymentMethods={paymentMethods || []}
                cardConnectionId={cardConnectionId}
                directDebitConnectionId={directDebitConnectionId}
                onRefetch={() => refetch()}
              />
            </div>
          </div>

          {/* Billing Information */}
          <div>
            <h2 className="text-xs font-medium text-gray-500 mb-2">Billing information</h2>
            <div className="bg-white border border-gray-200 rounded p-4">
              <BillingInfo
                customer={customer}
                isEditing={isAddressEditing}
                setIsEditing={setIsAddressEditing}
              />
            </div>
          </div>
        </div>

        {/* Invoices Section */}
        <div className="mb-6">
          <h2 className="text-xs font-medium text-gray-500 mb-2">Invoices</h2>
          <div className="bg-white border border-gray-200 rounded p-4">
            <CustomerPortalInvoices />
          </div>
        </div>
      </div>
    </div>
  )
}
