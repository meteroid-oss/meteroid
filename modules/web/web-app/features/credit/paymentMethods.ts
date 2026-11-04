import {
  CustomerPaymentMethod,
  CustomerPaymentMethod_PaymentMethodTypeEnum,
} from '@/rpc/api/customers/v1/models_pb'

const capitalize = (value: string) => value[0].toUpperCase() + value.slice(1)

export const paymentMethodLabel = (method: CustomerPaymentMethod) =>
  method.paymentMethodType === CustomerPaymentMethod_PaymentMethodTypeEnum.CARD
    ? `${method.cardBrand ? capitalize(method.cardBrand) : 'Card'} •••• ${method.cardLast4 ?? ''}`.trim()
    : `Bank account${method.accountNumberHint ? ` ••••${method.accountNumberHint}` : ''}`
