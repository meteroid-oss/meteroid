import { Building } from 'lucide-react'
import { useState } from 'react'

import { CardBrandLogo } from '@/features/checkout/components/CardBrandLogo'
import {
  CustomerPaymentMethod,
  CustomerPaymentMethod_PaymentMethodTypeEnum,
} from '@/rpc/api/customers/v1/models_pb'

import { AddPaymentMethodDialog } from './AddPaymentMethodDialog'

interface CustomerPortalPaymentMethodsProps {
  paymentMethods: CustomerPaymentMethod[]
  cardConnectionId?: string
  directDebitConnectionId?: string
  onRefetch?: () => void
}

export const CustomerPortalPaymentMethods = ({
  paymentMethods,
  cardConnectionId,
  directDebitConnectionId,
  onRefetch,
}: CustomerPortalPaymentMethodsProps) => {
  const [isDialogOpen, setIsDialogOpen] = useState(false)

  const handleAddPaymentMethod = () => {
    setIsDialogOpen(true)
  }

  const handleSuccess = () => {
    // Refetch payment methods after successful addition
    if (onRefetch) {
      onRefetch()
    }
  }

  // Check if any payment method connections are configured
  const hasPaymentConnections = !!(cardConnectionId || directDebitConnectionId)

  return (
    <>
      {paymentMethods.length === 0 ? (
        <div className="text-center py-3">
          <p className="text-xs text-gray-500 mb-2">No payment method on file</p>
          {hasPaymentConnections && (
            <button
              onClick={handleAddPaymentMethod}
              className="text-xs text-gray-600 hover:text-gray-900 font-medium"
            >
              + Add payment method
            </button>
          )}
        </div>
      ) : (
        <div className="space-y-2">
          {paymentMethods.map((method, index) => {
            const isCard =
              method.paymentMethodType === CustomerPaymentMethod_PaymentMethodTypeEnum.CARD
            const isDefault = index === 0 // First one is default

            return (
              <div key={method.id} className="flex items-center justify-between gap-3 text-sm">
                <div className="flex items-center gap-2.5 min-w-0">
                  {isCard ? (
                    <>
                      <div className="flex-shrink-0">
                        {method.cardBrand && <CardBrandLogo brand={method.cardBrand} />}
                      </div>
                      <div className="min-w-0">
                        <div className="text-gray-900 font-medium truncate">
                          •••• {method.cardLast4}
                        </div>
                        <div className="text-xs text-gray-500">
                          Expires {method.cardExpMonth?.toString().padStart(2, '0')}/
                          {method.cardExpYear}
                        </div>
                      </div>
                    </>
                  ) : (
                    <>
                      <Building size={16} className="text-gray-500 flex-shrink-0" />
                      <div className="min-w-0">
                        <div className="text-gray-900 font-medium truncate">Bank account</div>
                        <div className="text-xs text-gray-500">
                          {method.accountNumberHint && `••••${method.accountNumberHint}`}
                        </div>
                      </div>
                    </>
                  )}
                </div>
                {isDefault && (
                  <span className="text-xs text-gray-500 bg-gray-100 px-2 py-0.5 rounded flex-shrink-0">
                    Default
                  </span>
                )}
              </div>
            )
          })}

          {hasPaymentConnections && (
            <button
              onClick={handleAddPaymentMethod}
              className="text-xs text-gray-600 hover:text-gray-900 font-medium mt-2"
            >
              + Add payment method
            </button>
          )}
        </div>
      )}

      <AddPaymentMethodDialog
        open={isDialogOpen}
        onOpenChange={setIsDialogOpen}
        onSuccess={handleSuccess}
        cardConnectionId={cardConnectionId}
        directDebitConnectionId={directDebitConnectionId}
      />
    </>
  )
}
