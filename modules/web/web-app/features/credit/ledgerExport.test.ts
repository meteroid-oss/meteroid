import { describe, expect, it } from 'vitest'

import { customerLedgerCsvUrl } from './ledgerExport'

describe('customerLedgerCsvUrl', () => {
  it('points at the streamed download, the token escaped', () => {
    expect(customerLedgerCsvUrl('https://api.test', 'cus_123', 'a.b+c/d')).toBe(
      'https://api.test/files/v1/credit-ledger/cus_123?token=a.b%2Bc%2Fd'
    )
  })
})
