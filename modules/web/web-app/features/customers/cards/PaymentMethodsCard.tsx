import { Building } from 'lucide-react'

import { CardBrandLogo } from '@/features/checkout/components/CardBrandLogo'
import {
  CustomerPaymentMethod,
  CustomerPaymentMethod_PaymentMethodTypeEnum,
} from '@/rpc/api/customers/v1/models_pb'

interface PaymentMethodsCardProps {
  paymentMethods: CustomerPaymentMethod[]
  currentPaymentMethodId?: string
}

export const PaymentMethodsCard = ({
  paymentMethods,
  currentPaymentMethodId,
}: PaymentMethodsCardProps) => {
  if (!paymentMethods || paymentMethods.length === 0) {
    return (
      <div className="text-[13px] text-muted-foreground">No payment methods on file</div>
    )
  }

  return (
    <div className="space-y-3">
      {paymentMethods.map(method => {
        const isCard =
          method.paymentMethodType === CustomerPaymentMethod_PaymentMethodTypeEnum.CARD
        const isDefault = method.id === currentPaymentMethodId

        return (
          <div key={method.id} className="flex items-center justify-between">
            <div className="flex items-center gap-2.5">
              {isCard ? (
                <>
                  {method.cardBrand && <CardBrandLogo brand={method.cardBrand} />}
                  <div>
                    <div className="text-[13px] font-medium">•••• {method.cardLast4}</div>
                    {method.cardExpMonth && method.cardExpYear && (
                      <div className="text-xs text-muted-foreground">
                        Expires {method.cardExpMonth.toString().padStart(2, '0')}/
                        {method.cardExpYear}
                      </div>
                    )}
                  </div>
                </>
              ) : (
                <>
                  <Building size={16} className="text-muted-foreground" />
                  <div>
                    <div className="text-[13px] font-medium">Bank account</div>
                    {method.accountNumberHint && (
                      <div className="text-xs text-muted-foreground">
                        ••••{method.accountNumberHint}
                      </div>
                    )}
                  </div>
                </>
              )}
            </div>
            {isDefault && (
              <span className="text-xs text-muted-foreground bg-secondary px-2 py-0.5 rounded">
                Default
              </span>
            )}
          </div>
        )
      })}
    </div>
  )
}
