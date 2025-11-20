import { Building, Copy } from 'lucide-react'
import { useState } from 'react'

import type { BankAccount } from '@/rpc/api/bankaccounts/v1/models_pb'

interface BankTransferInfoProps {
  bankAccount: BankAccount
  invoiceNumber?: string
  customerName?: string
}

/**
 * Display bank transfer information for manual payment
 */
export const BankTransferInfo: React.FC<BankTransferInfoProps> = ({
  bankAccount,
  invoiceNumber,
  customerName,
}) => {
  const [copiedField, setCopiedField] = useState<string | null>(null)

  const copyToClipboard = (text: string, field: string) => {
    navigator.clipboard.writeText(text)
    setCopiedField(field)
    setTimeout(() => setCopiedField(null), 2000)
  }

  const renderCopyButton = (value: string, field: string) => (
    <button
      type="button"
      onClick={() => copyToClipboard(value, field)}
      className="ml-2 p-1 text-gray-500 hover:text-gray-700 transition-colors"
      title="Copy to clipboard"
    >
      {copiedField === field ? (
        <span className="text-xs text-green-600">✓</span>
      ) : (
        <Copy size={14} />
      )}
    </button>
  )

  const bankData = bankAccount.data
  if (!bankData) return null

  const renderAccountDetails = () => {
    if (!bankData.format?.value) return null

    switch (bankData.format.case) {
      case 'ibanBicSwift':
        return (
          <>
            <div className="flex items-center justify-between py-3 border-b">
              <span className="text-sm text-gray-600">IBAN</span>
              <div className="flex items-center">
                <span className="font-mono text-sm font-medium">
                  {bankData.format.value.iban}
                </span>
                {renderCopyButton(bankData.format.value.iban, 'iban')}
              </div>
            </div>
            {bankData.format.value.bicSwift && (
              <div className="flex items-center justify-between py-3 border-b">
                <span className="text-sm text-gray-600">BIC/SWIFT</span>
                <div className="flex items-center">
                  <span className="font-mono text-sm font-medium">
                    {bankData.format.value.bicSwift}
                  </span>
                  {renderCopyButton(bankData.format.value.bicSwift, 'bic')}
                </div>
              </div>
            )}
          </>
        )

      case 'accountNumberBicSwift':
        return (
          <>
            <div className="flex items-center justify-between py-3 border-b">
              <span className="text-sm text-gray-600">Account Number</span>
              <div className="flex items-center">
                <span className="font-mono text-sm font-medium">
                  {bankData.format.value.accountNumber}
                </span>
                {renderCopyButton(bankData.format.value.accountNumber, 'account')}
              </div>
            </div>
            <div className="flex items-center justify-between py-3 border-b">
              <span className="text-sm text-gray-600">BIC/SWIFT</span>
              <div className="flex items-center">
                <span className="font-mono text-sm font-medium">
                  {bankData.format.value.bicSwift}
                </span>
                {renderCopyButton(bankData.format.value.bicSwift, 'bic')}
              </div>
            </div>
          </>
        )

      case 'accountNumberRoutingNumber':
        return (
          <>
            <div className="flex items-center justify-between py-3 border-b">
              <span className="text-sm text-gray-600">Account Number</span>
              <div className="flex items-center">
                <span className="font-mono text-sm font-medium">
                  {bankData.format.value.accountNumber}
                </span>
                {renderCopyButton(bankData.format.value.accountNumber, 'account')}
              </div>
            </div>
            <div className="flex items-center justify-between py-3 border-b">
              <span className="text-sm text-gray-600">Routing Number</span>
              <div className="flex items-center">
                <span className="font-mono text-sm font-medium">
                  {bankData.format.value.routingNumber}
                </span>
                {renderCopyButton(bankData.format.value.routingNumber, 'routing')}
              </div>
            </div>
          </>
        )

      case 'sortCodeAccountNumber':
        return (
          <>
            <div className="flex items-center justify-between py-3 border-b">
              <span className="text-sm text-gray-600">Sort Code</span>
              <div className="flex items-center">
                <span className="font-mono text-sm font-medium">
                  {bankData.format.value.sortCode}
                </span>
                {renderCopyButton(bankData.format.value.sortCode, 'sort')}
              </div>
            </div>
            <div className="flex items-center justify-between py-3 border-b">
              <span className="text-sm text-gray-600">Account Number</span>
              <div className="flex items-center">
                <span className="font-mono text-sm font-medium">
                  {bankData.format.value.accountNumber}
                </span>
                {renderCopyButton(bankData.format.value.accountNumber, 'account')}
              </div>
            </div>
          </>
        )

      default:
        return null
    }
  }

  const referenceText = `${invoiceNumber ? `Invoice ${invoiceNumber}` : ''}${customerName ? ` - ${customerName}` : ''}`.trim()

  return (
    <div className="bg-white border border-gray-200 rounded-lg p-6 mb-6">
      <div className="flex items-center mb-4">
        <Building size={20} className="text-blue-600 mr-2" />
        <h3 className="text-lg font-semibold">Bank Transfer Details</h3>
      </div>

      <p className="text-sm text-gray-600 mb-4">
        You can pay this invoice by bank transfer using the details below
      </p>

      <div className="space-y-0">
        <div className="flex items-center justify-between py-3 border-b">
          <span className="text-sm text-gray-600">Bank Name</span>
          <span className="text-sm font-medium">{bankData.bankName}</span>
        </div>

        <div className="flex items-center justify-between py-3 border-b">
          <span className="text-sm text-gray-600">Country</span>
          <span className="text-sm font-medium">{bankData.country}</span>
        </div>

        {renderAccountDetails()}

        {referenceText && (
          <div className="flex items-center justify-between py-3">
            <span className="text-sm text-gray-600">Payment Reference</span>
            <div className="flex items-center">
              <span className="font-mono text-sm font-medium">{referenceText}</span>
              {renderCopyButton(referenceText, 'reference')}
            </div>
          </div>
        )}
      </div>

      <div className="mt-4 p-3 bg-blue-50 rounded-md">
        <p className="text-xs text-blue-800">
          <strong>Important:</strong> Please include the payment reference in your transfer to
          ensure proper allocation of your payment.
        </p>
      </div>
    </div>
  )
}