import { AlertCircle, Ban, CheckCircle, Info, XCircle } from 'lucide-react'

interface ReadonlyPaymentViewProps {
  reason:
    | 'already_paid'
    | 'voided'
    | 'cancelled'
    | 'uncollectible'
    | 'no_payment_methods'
    | 'external_payment'
    | 'already_active'
    | 'draft_invoice'
  title?: string
  message?: string
  children?: React.ReactNode
}

export const ReadonlyPaymentView: React.FC<ReadonlyPaymentViewProps> = ({
  reason,
  title,
  message,
  children,
}) => {
  const config = getReasonConfig(reason)

  return (
    <div className="py-6 mt-6">
      <div className="flex items-start">
        <div className={`flex-shrink-0 ${config.iconColor}`}>{config.icon}</div>
        <div className="ml-4 flex-1">
          <h3 className="text-sm font-medium text-gray-900">{title || config.title}</h3>
          <div className="mt-2 text-sm text-gray-600">{message || config.message}</div>

          {children && <div className="mt-4">{children}</div>}
        </div>
      </div>
    </div>
  )
}

function getReasonConfig(reason: ReadonlyPaymentViewProps['reason']) {
  const configs: Record<
    ReadonlyPaymentViewProps['reason'],
    {
      icon: React.ReactNode
      iconColor: string
      title: string
      message: string
    }
  > = {
    already_paid: {
      icon: <CheckCircle className="h-6 w-6" />,
      iconColor: 'text-green-600',
      title: 'Payment Completed',
      message: 'Thank you for your payment!',
    },
    already_active: {
      icon: <CheckCircle className="h-6 w-6" />,
      iconColor: 'text-green-600',
      title: 'Subscription Active',
      message: 'This subscription is already active. No payment is required.',
    },
    voided: {
      icon: <Ban className="h-6 w-6" />,
      iconColor: 'text-gray-500',
      title: 'Invoice Voided',
      message: 'This invoice has been voided and is no longer payable.',
    },
    cancelled: {
      icon: <XCircle className="h-6 w-6" />,
      iconColor: 'text-gray-500',
      title: 'Subscription Cancelled',
      message: 'This subscription has been cancelled and is no longer available.',
    },
    uncollectible: {
      icon: <AlertCircle className="h-6 w-6" />,
      iconColor: 'text-orange-600',
      title: 'Invoice Uncollectible',
      message:
        'This invoice has been marked as uncollectible. Please contact support if you have questions.',
    },
    no_payment_methods: {
      icon: <Info className="h-6 w-6" />,
      iconColor: 'text-blue-600',
      title: 'No Payment Methods Available',
      message: 'There are currently no payment methods configured. Please contact support.',
    },
    external_payment: {
      icon: <Info className="h-6 w-6" />,
      iconColor: 'text-blue-600',
      title: 'External Payment',
      message:
        'Payment for this invoice is processed externally. Please refer to your agreement or contact support for payment instructions.',
    },
    draft_invoice: {
      icon: <Info className="h-6 w-6" />,
      iconColor: 'text-gray-500',
      title: 'Draft Invoice',
      message:
        'This invoice is still in draft status and cannot be paid yet. It will be available for payment once finalized.',
    },
  }

  return configs[reason]
}
