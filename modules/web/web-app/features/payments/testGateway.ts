import { SimulateTestGatewayEventRequest_TestGatewayEvent as TestGatewayEvent } from '@/rpc/api/invoices/v1/invoices_pb'
import { Transaction, Transaction_PaymentStatusEnum } from '@/rpc/api/invoices/v1/models_pb'

/** Provider ids the test gateway mints; the only way a transaction reveals which connector charged it. */
const TEST_GATEWAY_INTENT_PREFIX = 'mock_pi_'

export const isTestGatewayTransaction = (transaction: Transaction): boolean =>
  !!transaction.providerTransactionId?.startsWith(TEST_GATEWAY_INTENT_PREFIX)

export interface TestGatewayAction {
  event: TestGatewayEvent
  label: string
  /** Asked before the event is sent; absent for the reversible pending outcomes. */
  confirm?: string
}

/** Events a merchant can push for this transaction, mirroring what the backend accepts. */
export const testGatewayActionsFor = (transaction: Transaction): TestGatewayAction[] => {
  if (!isTestGatewayTransaction(transaction)) return []
  switch (transaction.status) {
    case Transaction_PaymentStatusEnum.PENDING:
      return [
        { event: TestGatewayEvent.SUCCEEDED, label: 'Settle' },
        { event: TestGatewayEvent.FAILED, label: 'Fail' },
      ]
    case Transaction_PaymentStatusEnum.SETTLED:
      return [
        {
          event: TestGatewayEvent.REFUNDED,
          label: 'Refund (provider-side)',
          confirm:
            'Simulates a refund issued from the provider dashboard. The invoice stays paid and a refund row is recorded against this payment.',
        },
        {
          event: TestGatewayEvent.CHARGED_BACK,
          label: 'Chargeback',
          confirm:
            'Simulates the payer charging the payment back. The payment is lost to the dispute; the invoice stays paid.',
        },
      ]
    default:
      return []
  }
}
