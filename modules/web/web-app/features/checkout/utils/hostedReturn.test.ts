import { describe, expect, it } from 'vitest'

import { hostedReturnTarget, withRestoredToken } from './hostedReturn'

describe('hostedReturnTarget', () => {
  it('never hands the page token to the provider, and marks the URL for its restore', () => {
    expect(
      hostedReturnTarget(
        'https://billing.example.com/portal/customer?token=tok_1&page=settings&hosted_status=failed&hosted_error=x'
      )
    ).toBe('https://billing.example.com/portal/customer?page=settings&hosted_return=1')
  })

  it('drops every credential, user info and fragment', () => {
    expect(
      hostedReturnTarget(
        'https://u:p@billing.example.com/portal/invoice-payment/inv_1?access_token=a&session_id=b&api_key=c&sig=d&code=e&theme=dark#tok'
      )
    ).toBe('https://billing.example.com/portal/invoice-payment/inv_1?theme=dark&hosted_return=1')
  })

  it('leaves a URL without credentials unmarked', () => {
    expect(hostedReturnTarget('https://billing.example.com/checkout?plan=pro')).toBe(
      'https://billing.example.com/checkout?plan=pro'
    )
  })
})

describe('withRestoredToken', () => {
  const returned =
    'https://billing.example.com/portal/customer?page=settings&hosted_return=1&hosted_status=ok'

  it('puts the stored token back on a marked return and drops the marker', () => {
    expect(withRestoredToken(returned, 'tok_1')).toBe(
      'https://billing.example.com/portal/customer?page=settings&hosted_status=ok&token=tok_1'
    )
  })

  it('drops the marker without a stored token, so the page shows its expired state', () => {
    expect(withRestoredToken(returned, null)).toBe(
      'https://billing.example.com/portal/customer?page=settings&hosted_status=ok'
    )
  })

  it('changes nothing on an unmarked page or one that already has its token', () => {
    expect(withRestoredToken('https://billing.example.com/portal/customer?token=a', 'b')).toBeNull()
    expect(
      withRestoredToken('https://billing.example.com/checkout?token=a&hosted_return=1', 'b')
    ).toBe('https://billing.example.com/checkout?token=a')
  })
})
