import { timestampDate } from '@bufbuild/protobuf/wkt'
import { createConnectQueryKey } from '@connectrpc/connect-query'

import { slotItems } from '@/edition'
import { parseAndFormatDate, parseAndFormatDateTime } from '@/lib/utils/date'
import { getUpcomingInvoice } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { CreditGrantSource, LedgerEntryType } from '@/rpc/api/wallets/v1/models_pb'
import {
  getCustomerCreditActivity,
  listCustomerLedgerEntries,
  listCustomerWallets,
  listLedgerEntries,
  listWalletGrants,
  listWalletSummaries,
} from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { CustomerLedgerEntry } from '@/rpc/api/wallets/v1/models_pb'
import type { DescMethodUnary } from '@bufbuild/protobuf'
import type { Timestamp } from '@bufbuild/protobuf/wkt'
import type { QueryClient } from '@tanstack/react-query'

export const grantSourceLabel: Record<CreditGrantSource, string> = {
  [CreditGrantSource.UNSPECIFIED]: 'Unknown',
  [CreditGrantSource.CUSTOMER_BALANCE]: 'Previous balance',
  [CreditGrantSource.PURCHASE]: 'Purchased credit',
  [CreditGrantSource.INCLUDED]: 'Included with plan',
  [CreditGrantSource.CREDIT_NOTE]: 'Credit note',
  [CreditGrantSource.NEGATIVE_INVOICE]: 'Credit from an invoice',
  [CreditGrantSource.INVOICE_VOID]: 'Returned from a voided invoice',
  [CreditGrantSource.MANUAL]: 'Added by a team member',
  [CreditGrantSource.API]: 'Added through the API',
  [CreditGrantSource.ROLLOVER]: 'Rolled over from last period',
  [CreditGrantSource.COUPON]: 'Free credit',
  [CreditGrantSource.OVERPAYMENT]: 'Overpayment',
}

export const ledgerEntryLabel: Record<LedgerEntryType, string> = {
  [LedgerEntryType.UNSPECIFIED]: 'Movement',
  [LedgerEntryType.GRANTED]: 'Credit added',
  [LedgerEntryType.APPLIED]: 'Applied to invoice',
  [LedgerEntryType.RETURNED]: 'Credit returned',
  [LedgerEntryType.REMOVED]: 'Credit removed',
  [LedgerEntryType.EXPIRED]: 'Credit expired',
  [LedgerEntryType.ROLLED_OVER]: 'Rolled over',
}

/** What a ledger movement was, as the server words it for the dashboard and the portal. */
export const ledgerMovementLabel = (row: CustomerLedgerEntry): string =>
  row.label || ledgerEntryLabel[row.entry?.type ?? LedgerEntryType.UNSPECIFIED]

export const formatDate = (timestamp?: Timestamp) =>
  timestamp ? parseAndFormatDate(timestampDate(timestamp).toISOString()) : ''

export const formatDateTime = (timestamp?: Timestamp) =>
  timestamp ? parseAndFormatDateTime(timestampDate(timestamp).toISOString()) : ''

// With what credit changes downstream: its coverage, and what the next invoices draw.
const WALLET_QUERIES: DescMethodUnary[] = [
  listCustomerWallets,
  listLedgerEntries,
  listWalletSummaries,
  listCustomerLedgerEntries,
  getCustomerCreditActivity,
  listWalletGrants,
  getUpcomingInvoice,
]

export const invalidateWallets = (queryClient: QueryClient) =>
  Promise.all(
    [...WALLET_QUERIES, ...slotItems('wallets.queries')].map(schema =>
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema, cardinality: undefined }),
      })
    )
  )
