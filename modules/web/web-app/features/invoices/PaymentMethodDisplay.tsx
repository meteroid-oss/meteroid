import { Building, CreditCard, HelpCircle, Wallet } from 'lucide-react'

import {
  PaymentMethodInfo,
  PaymentMethodInfo_PaymentMethodTypeEnum,
} from '@/rpc/api/invoices/v1/models_pb'

interface PaymentMethodDisplayProps {
  paymentMethodInfo?: PaymentMethodInfo
  compact?: boolean
}

export const PaymentMethodDisplay = ({
  paymentMethodInfo,
  compact = false,
}: PaymentMethodDisplayProps) => {
  if (!paymentMethodInfo) {
    return <span className="text-sm">Manual</span>
  }

  const getIcon = () => {
    switch (paymentMethodInfo.paymentMethodType) {
      case PaymentMethodInfo_PaymentMethodTypeEnum.CARD:
        return <CreditCard className="w-4 h-4" />
      case PaymentMethodInfo_PaymentMethodTypeEnum.BANK_TRANSFER:
        return <Building className="w-4 h-4" />
      case PaymentMethodInfo_PaymentMethodTypeEnum.WALLET:
        return <Wallet className="w-4 h-4" />
      default:
        return <HelpCircle className="w-4 h-4" />
    }
  }

  const getDisplayText = () => {
    switch (paymentMethodInfo.paymentMethodType) {
      case PaymentMethodInfo_PaymentMethodTypeEnum.CARD:
        if (paymentMethodInfo.cardBrand && paymentMethodInfo.cardLast4) {
          return `${paymentMethodInfo.cardBrand} •••• ${paymentMethodInfo.cardLast4}`
        }
        return 'Card'

      case PaymentMethodInfo_PaymentMethodTypeEnum.BANK_TRANSFER:
        if (paymentMethodInfo.accountNumberHint) {
          return `Bank •••• ${paymentMethodInfo.accountNumberHint}`
        }
        return 'Bank Transfer'

      case PaymentMethodInfo_PaymentMethodTypeEnum.WALLET:
        return 'Digital Wallet'

      default:
        return 'Payment Method'
    }
  }

  if (compact) {
    return (
      <div className="flex items-center gap-1.5 text-sm">
        {getIcon()}
        <span>{getDisplayText()}</span>
      </div>
    )
  }

  return (
    <div className="flex items-center gap-2">
      {getIcon()}
      <span className="font-medium">{getDisplayText()}</span>
    </div>
  )
}

export default PaymentMethodDisplay
