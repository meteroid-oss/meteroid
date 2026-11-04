import { describe, expect, it } from 'vitest'

import {
  openingPurchaseKey,
  returnedWallet,
  topUpCreditLanded,
  topUpReturnUrl,
  withStoredPortalToken,
} from './topUpCheckout'

describe('topUpReturnUrl', () => {
  it('returns to the full portal Credit page on the purchase, without the token', () => {
    expect(
      topUpReturnUrl(
        'https://billing.example.com/portal/customer?token=tok_1&embed=portal&nav=host&page=overview&sub=s1',
        'wal_1'
      )
    ).toBe('https://billing.example.com/portal/customer?page=balance&topUpReturn=wal_1')
  })

  it('drops every credential and embed parameter, user info and fragment, keeping the theme', () => {
    expect(
      topUpReturnUrl(
        'https://u:p@billing.example.com/portal/customer?access_token=a&Portal-Token=b&session_id=c&api_key=d&sig=e&code=f&subscription=s&count=3&theme=dark&accent=%23112233#tok',
        'new-EUR'
      )
    ).toBe(
      'https://billing.example.com/portal/customer?theme=dark&accent=%23112233&page=balance&topUpReturn=new-EUR'
    )
  })
})

describe('withStoredPortalToken', () => {
  const back = 'https://billing.example.com/portal/customer?page=balance&topUpReturn=wal_1'

  it('re-attaches the token the tab kept to a checkout return', () => {
    expect(withStoredPortalToken(back, 'tok_1')).toBe(`${back}&token=tok_1`)
  })

  it('leaves the URL alone without a stored token, with a token, or off a checkout return', () => {
    expect(withStoredPortalToken(back, null)).toBeNull()
    expect(withStoredPortalToken(`${back}&token=tok_url`, 'tok_1')).toBeNull()
    expect(
      withStoredPortalToken('https://billing.example.com/portal/customer?page=balance', 'tok_1')
    ).toBeNull()
  })
})

describe('returnedWallet', () => {
  const money = { id: 'wal_eur', currency: 'EUR', creditUnit: undefined }
  const units = { id: 'wal_api', currency: 'EUR', creditUnit: { code: 'EUR' } as never }

  it('finds the wallet by id, or the money wallet an opening purchase opened', () => {
    expect(returnedWallet('wal_api', [units, money])).toBe(units)
    expect(returnedWallet(openingPurchaseKey('eur'), [units, money])).toBe(money)
    expect(returnedWallet('new-USD', [units, money])).toBeUndefined()
    expect(returnedWallet(null, [money])).toBeUndefined()
  })
})

describe('topUpCreditLanded', () => {
  const wallet = (available: bigint, liveStale = false, id = 'wal_1') => ({
    id,
    currency: 'EUR',
    creditUnit: undefined,
    available,
    liveStale,
  })

  it('waits until the bought credit is on the wallet and the live balance caught up', () => {
    expect(topUpCreditLanded([wallet(6_500n)], 'wal_1', 6_500n, 1_000n)).toBe(false)
    expect(topUpCreditLanded([wallet(7_500n, true)], 'wal_1', 6_500n, 1_000n)).toBe(false)
    expect(topUpCreditLanded([wallet(7_500n)], 'wal_1', 6_500n, 1_000n)).toBe(true)
  })

  it('ignores another movement of the wallet before the credit lands', () => {
    expect(topUpCreditLanded([wallet(6_000n)], 'wal_1', 6_500n, 1_000n)).toBe(false)
    expect(topUpCreditLanded([wallet(7_000n)], 'wal_1', 6_500n, 1_000n)).toBe(false)
  })

  it('follows a purchase that opened the money wallet by its currency', () => {
    expect(
      topUpCreditLanded([wallet(1_000n, false, 'wal_new')], openingPurchaseKey('eur'), 0n, 1_000n)
    ).toBe(true)
    expect(topUpCreditLanded([], openingPurchaseKey('EUR'), 0n, 1_000n)).toBe(false)
  })
})
