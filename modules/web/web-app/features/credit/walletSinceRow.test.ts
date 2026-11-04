import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { CreditGrantType, WalletSchema } from '@/rpc/api/wallets/v1/models_pb'

import { usedTaxBasis } from './WalletSinceRow'

const unit = { id: 'cu_1', code: 'TOK', name: 'Token', pluralName: 'Tokens', precision: 0 }

describe('usedTaxBasis', () => {
  const money = (grantTypes: CreditGrantType[]) =>
    usedTaxBasis({ creditUnit: undefined, grantTypes })

  it('counts paid credit tax included and promotional credit before tax', () => {
    expect(money([CreditGrantType.PAID])).toBe('incl')
    expect(money([CreditGrantType.PROMOTIONAL])).toBe('excl')
    expect(money([CreditGrantType.PAID, CreditGrantType.PROMOTIONAL])).toBe('mixed')
    expect(money([])).toBeUndefined()
  })

  it('says nothing about tax for credits of a unit', () => {
    expect(
      usedTaxBasis({
        creditUnit: create(WalletSchema, { creditUnit: unit }).creditUnit,
        grantTypes: [],
      })
    ).toBeUndefined()
  })
})
