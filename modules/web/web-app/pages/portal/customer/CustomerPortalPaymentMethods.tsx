import { Button } from '@md/ui'
import { Building, CreditCard, Plus } from 'lucide-react'
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
      <div className="text-sm">
        <div className="text-sm font-medium mb-2">Payment methods</div>

        {paymentMethods.length === 0 ? (
          <div className="border border-gray-200 rounded-lg p-6 text-center">
            <div className="text-sm text-muted-foreground mb-3">No payment methods saved</div>
            {hasPaymentConnections && (
              <Button
                size="sm"
                onClick={handleAddPaymentMethod}
                className="bg-blue-600 hover:bg-blue-700"
              >
                <Plus size={16} className="mr-2" />
                Add payment method
              </Button>
            )}
          </div>
        ) : (
        <div className="space-y-2">
          {paymentMethods.map(method => {
            const isCard =
              method.paymentMethodType === CustomerPaymentMethod_PaymentMethodTypeEnum.CARD
            const isDefault = false // TODO

            return (
              <div
                key={method.id}
                className="relative flex items-center p-4 border border-gray-200 rounded-lg"
              >
                {isCard ? (
                  <>
                    <CreditCard size={20} className="mr-3 text-gray-500" />
                    <div className="flex-1">
                      <div className="font-medium text-sm">
                        {method.cardBrand} •••• {method.cardLast4}
                      </div>
                      <div className="text-xs text-gray-500">
                        Expires {method.cardExpMonth?.toString().padStart(2, '0')}/
                        {method.cardExpYear?.toString().slice(-2)}
                      </div>
                    </div>
                    {method.cardBrand && (
                      <div className="ml-auto">
                        <CardBrandLogo brand={method.cardBrand} />
                      </div>
                    )}
                  </>
                ) : (
                  <>
                    <Building size={20} className="mr-3 text-gray-500" />
                    <div className="flex-1">
                      <div className="font-medium text-sm">Bank account</div>
                      <div className="text-xs text-gray-500">
                        {method.accountNumberHint && `••••${method.accountNumberHint}`}
                      </div>
                    </div>
                  </>
                )}

                {isDefault && (
                  <div className="absolute top-2 right-2 bg-blue-100 text-blue-800 text-xs rounded px-2 py-0.5">
                    Default
                  </div>
                )}
              </div>
            )
          })}

          {hasPaymentConnections && (
            <Button
              size="sm"
              variant="ghost"
              onClick={handleAddPaymentMethod}
              className="w-full text-blue-600 hover:text-blue-700 hover:bg-blue-50"
            >
              <Plus size={16} className="mr-2" />
              Add another payment method
            </Button>
          )}
        </div>
      )}
      </div>

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
