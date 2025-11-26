import { ArrowLeft } from 'lucide-react'
import { useParams, useSearchParams } from 'react-router-dom'

/**
 * Portal page for subscription detail view
 * TODO: Implement full subscription detail view with:
 * - Subscription components and pricing
 * - Upcoming invoice preview
 * - Usage tracking
 * - Invoice history for this subscription
 * - Cancellation UI
 */
export const PortalSubscription = () => {
  const { subscriptionId } = useParams<{ subscriptionId: string }>()
  const [searchParams] = useSearchParams()
  const token = searchParams.get('token')

  const handleBackToPortal = () => {
    if (token) {
      window.location.href = `/portal/customer?token=${token}`
    } else {
      window.history.back()
    }
  }

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Header */}
      <div className="bg-white border-b border-gray-200">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <button
            onClick={handleBackToPortal}
            className="flex items-center text-sm text-gray-600 hover:text-gray-900 mb-4"
          >
            <ArrowLeft size={16} className="mr-2" />
            Back to Portal
          </button>
          <h1 className="text-2xl font-bold text-gray-900">Subscription Details</h1>
        </div>
      </div>

      {/* Content */}
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <div className="bg-white rounded-lg shadow-sm border border-gray-200 p-8">
          <div className="text-center text-gray-600">
            <h2 className="text-lg font-semibold text-gray-900 mb-2">
              Subscription View Coming Soon
            </h2>
            <p className="text-sm">
              Detailed subscription view with components, usage, invoices, and cancellation options
              will be available here.
            </p>
            <p className="text-xs text-gray-500 mt-4">Subscription ID: {subscriptionId}</p>
          </div>
        </div>
      </div>
    </div>
  )
}
