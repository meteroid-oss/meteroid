import { Building, Check, CreditCard } from 'lucide-react'
import React from 'react'
import StripeCardForm from './StripeCardForm'
import {
  BankAccountPaymentMethod,
  CardPaymentMethod,
  PaymentMethodOption,
  PaymentMethodSelection,
} from './types'

interface PaymentMethodSelectorProps {
  savedMethods: (CardPaymentMethod | BankAccountPaymentMethod)[]
  availableMethods: PaymentMethodOption[]
  selectedMethod: PaymentMethodSelection | null
  onSelectMethod: (method: PaymentMethodSelection) => void
  savePaymentMethod: boolean
  onToggleSaveMethod: (save: boolean) => void
}

const PaymentMethodSelector: React.FC<PaymentMethodSelectorProps> = ({
  savedMethods,
  availableMethods,
  selectedMethod,
  onSelectMethod,
  savePaymentMethod,
  onToggleSaveMethod,
}) => {
  const isSelected = (type: 'saved' | 'new', id: string): boolean => {
    if (!selectedMethod) return false

    if (type === 'saved' && selectedMethod.type === 'saved') {
      return selectedMethod.id === id
    }

    if (type === 'new' && selectedMethod.type === 'new') {
      return selectedMethod.methodId === id
    }

    return false
  }

  // Determine if we need to show Stripe Elements
  const showStripeCardForm = (): boolean => {
    if (!selectedMethod || selectedMethod.type !== 'new') return false

    const method = availableMethods.find(m => m.id === selectedMethod.methodId)
    return !!method && method.id === 'card' && method.provider.type === 'stripe'
  }

  return (
    <div>
      {/* Saved payment methods */}
      {savedMethods.length > 0 && (
        <div className="space-y-2 mb-4">
          {savedMethods.map(method => (
            <label
              key={method.id}
              className={`block relative rounded-lg border transition-all cursor-pointer p-3 ${
                isSelected('saved', method.id)
                  ? 'border-blue-600 ring-1 ring-blue-50'
                  : 'border-gray-200 hover:border-gray-300'
              }`}
            >
              <input
                type="radio"
                name="paymentMethod"
                value={`saved_${method.id}`}
                checked={isSelected('saved', method.id)}
                onChange={() => onSelectMethod({ type: 'saved', id: method.id })}
                className="sr-only"
              />
              <div className="flex justify-between items-center">
                <div className="flex items-center">
                  <div
                    className={`w-4 h-4 rounded-full flex items-center justify-center ${
                      isSelected('saved', method.id) ? 'bg-blue-600' : 'border border-gray-300'
                    }`}
                  >
                    {isSelected('saved', method.id) && <Check size={10} className="text-white" />}
                  </div>
                  <div className="ml-3 flex items-center">
                    {method.type === 'card' ? (
                      <CreditCard size={16} className="text-gray-400 mr-3" />
                    ) : (
                      <Building size={16} className="text-gray-400 mr-3" />
                    )}
                    <div>
                      <div className="text-sm font-medium">
                        {method.type === 'card'
                          ? `${method.brand.charAt(0).toUpperCase() + method.brand.slice(1)} ending in ${method.last4}`
                          : `${method.bankName} ••••${method.last4}`}
                      </div>
                      <div className="text-xs text-gray-500">
                        {method.type === 'card'
                          ? `Expires ${method.expMonth}/${method.expYear}`
                          : 'Direct debit'}
                      </div>
                    </div>
                  </div>
                </div>

                {/* If this method has a discount, show it */}
                {method.metadata?.discount && (
                  <div className="text-xs px-2 py-0.5 bg-green-50 text-green-700 rounded-full">
                    Save ${method.metadata.discount}
                  </div>
                )}
              </div>
            </label>
          ))}
        </div>
      )}

      {/* New payment methods */}
      <div className="space-y-2">
        {availableMethods.map(method => (
          <label
            key={method.id}
            className={`block relative rounded-lg border transition-all cursor-pointer p-3 ${
              isSelected('new', method.id)
                ? 'border-blue-600 ring-1 ring-blue-50'
                : 'border-gray-200 hover:border-gray-300'
            }`}
          >
            <input
              type="radio"
              name="paymentMethod"
              value={`new_${method.id}`}
              checked={isSelected('new', method.id)}
              onChange={() =>
                onSelectMethod({
                  type: 'new',
                  methodId: method.id,
                  providerId: method.provider.id,
                })
              }
              className="sr-only"
            />
            <div className="flex items-center">
              <div
                className={`w-4 h-4 rounded-full flex items-center justify-center ${
                  isSelected('new', method.id) ? 'bg-blue-600' : 'border border-gray-300'
                }`}
              >
                {isSelected('new', method.id) && <Check size={10} className="text-white" />}
              </div>
              <div className="ml-3 flex items-center">
                {method.id === 'card' ? (
                  <CreditCard size={16} className="text-gray-400 mr-3" />
                ) : (
                  <Building size={16} className="text-gray-400 mr-3" />
                )}
                <div className="text-sm font-medium">
                  {method.id === 'card' ? 'Add new card' : 'Connect bank account'}
                </div>
              </div>

              {/* If this method has a discount, show it */}
              {method.discount && (
                <div className="ml-auto text-xs px-2 py-0.5 bg-green-50 text-green-700 rounded-full">
                  Save{' '}
                  {method.discount.type === 'percentage'
                    ? `${method.discount.amount}%`
                    : `$${method.discount.amount}`}
                </div>
              )}
            </div>
          </label>
        ))}
      </div>

      {/* Show Stripe Card Elements form if a new card with Stripe provider is selected */}
      {showStripeCardForm() && (
        <div className="mt-4 p-4 bg-gray-50 rounded-lg">
          <div className="mb-4 flex justify-between items-center">
            <div className="text-sm text-gray-500">Card details</div>
            <div className="flex space-x-1">
              <img src="/api/placeholder/24/16" alt="Visa" />
              <img src="/api/placeholder/24/16" alt="Mastercard" />
              <img src="/api/placeholder/24/16" alt="Amex" />
            </div>
          </div>

          <StripeCardForm />

          <label className="flex items-start cursor-pointer mt-3">
            <input
              type="checkbox"
              checked={savePaymentMethod}
              onChange={() => onToggleSaveMethod(!savePaymentMethod)}
              className="mt-0.5 h-4 w-4 text-blue-600 rounded border-gray-300"
            />
            <span className="ml-2 text-sm text-gray-600">Save card for future payments</span>
          </label>
        </div>
      )}
    </div>
  )
}

export default PaymentMethodSelector
