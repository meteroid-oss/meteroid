import { create } from '@bufbuild/protobuf'
import { useMutation } from '@connectrpc/connect-query'
import { useEffect, useRef, useState } from 'react'
import { z } from 'zod'

import {
  amountSchema,
  minorUnitsToInput,
  minorUnitsToInputAtPrecision,
  parseMinorUnits,
  parseMinorUnitsAtPrecision,
  topUpPresets,
} from '@/features/credit/amounts'
import {
  creditsToMoneyMinor,
  formatCreditQuantity,
  formatCredits,
  quantitySchema,
} from '@/features/credit/unitSale'
import { useZodForm } from '@/hooks/useZodForm'
import { formatCurrency, formatCurrencyNoRounding } from '@/lib/utils/numbers'
import {
  getBalances,
  getTopUp,
  quoteTopUp,
  topUpWallet,
} from '@/rpc/portal/wallets/v1/wallets-PortalWalletsService_connectquery'
import { TopUpQuoteSchema, TopUpStatus } from '@/rpc/portal/wallets/v1/wallets_pb'

import { usePortalNav } from '../context'
import { pmLabel } from '../format'
import { useInvalidatePortal } from '../hooks'
import { Modal, ModalCloseButton, PButton } from '../primitives'
import { contactName, roundCreditPresets, topUpSettlesCopy } from '../screens/creditCopy'
import { customerError, errorStyle, fieldStyle, labelStyle } from '../walletFormat'

import {
  isEmbeddedPortal,
  openingPurchaseKey,
  topUpCreditLanded,
  topUpReturnUrl,
} from './topUpCheckout'

import type { CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'
import type { CreditUnitLabel, Wallet } from '@/rpc/portal/wallets/v1/models_pb'
import type { GetTopUpResponse, TopUpQuote } from '@/rpc/portal/wallets/v1/wallets_pb'

const buildSchema = (currency: string, creditUnit?: CreditUnitLabel) =>
  z.object({
    amount: creditUnit ? quantitySchema(creditUnit.precision) : amountSchema(currency),
    paymentMethodId: z.string(),
  })

// The portal never loads the tenant's unit catalog: credit amounts use the wallet's own precision.
const walletAmountCodec = (wallet: Wallet) => {
  const unit = wallet.creditUnit
  return unit
    ? {
        parse: (value: string) => parseMinorUnitsAtPrecision(value, unit.precision),
        toInput: (minor?: bigint) => minorUnitsToInputAtPrecision(minor, unit.precision),
      }
    : {
        parse: (value: string) => parseMinorUnits(value, wallet.currency),
        toInput: (minor?: bigint) => minorUnitsToInput(minor, wallet.currency),
      }
}

type Outcome = 'succeeded' | 'processing'

type Review = {
  walletId: string
  amount: bigint
  // Empty: a new card, paid on the hosted checkout page.
  paymentMethodId: string
  quote: TopUpQuote
  idempotencyKey: string
}

// A hosted checkout open in another tab: the purchase is followed until it settles.
type Checkout = { url: string; opened: boolean; timedOut: boolean }

const CHECKOUT_POLL_MS = 3000
const CHECKOUT_POLL_LIMIT_MS = 30 * 60 * 1000
// A succeeded charge funds the wallet a moment later, once its settlement is processed.
const LANDING_POLL_MS = 2000
const LANDING_POLL_LIMIT_MS = 60 * 1000

type PurchaseScope = Pick<Wallet, 'id' | 'currency' | 'creditUnit'>

// A sent purchase survives closing or reloading, so it is resumed under its key, never re-bought.
const pendingKey = (purchaseKey: string) => `meteroid:wallet-top-up:${purchaseKey}`
const openingKey = (scope: PurchaseScope) =>
  scope.creditUnit ? null : pendingKey(openingPurchaseKey(scope.currency))

const readPending = (key: string | null): Review | null => {
  if (!key) return null
  try {
    const stored = localStorage.getItem(key)
    if (!stored) return null
    const parsed = JSON.parse(stored)
    return {
      walletId: typeof parsed.walletId === 'string' ? parsed.walletId : '',
      amount: BigInt(parsed.amount),
      paymentMethodId: parsed.paymentMethodId,
      idempotencyKey: parsed.idempotencyKey,
      quote: create(TopUpQuoteSchema, {
        charge: BigInt(parsed.quote.charge),
        currency: parsed.quote.currency,
        net: BigInt(parsed.quote.net),
        tax: BigInt(parsed.quote.tax),
      }),
    }
  } catch {
    return null
  }
}

// A money wallet's purchase may have been sent before the wallet existed: it is that wallet's.
const loadPending = (scope: PurchaseScope): Review | null => {
  const own = scope.id ? readPending(pendingKey(scope.id)) : null
  if (own) return own
  const opening = readPending(openingKey(scope))
  return opening && { ...opening, walletId: opening.walletId || scope.id }
}

/** Whether a purchase sent for the wallet awaits its outcome, e.g. back from the hosted checkout. */
export const hasPendingTopUp = (scope: PurchaseScope) => loadPending(scope) !== null

const savePending = (scope: PurchaseScope, review: Review | null) => {
  const opening = openingKey(scope)
  try {
    if (review?.walletId) {
      localStorage.setItem(
        pendingKey(review.walletId),
        JSON.stringify(review, (_, value) => (typeof value === 'bigint' ? value.toString() : value))
      )
      if (opening) localStorage.removeItem(opening)
    } else if (review && opening) {
      localStorage.setItem(
        opening,
        JSON.stringify(review, (_, value) => (typeof value === 'bigint' ? value.toString() : value))
      )
    } else if (!review) {
      if (scope.id) localStorage.removeItem(pendingKey(scope.id))
      if (opening) localStorage.removeItem(opening)
    }
  } catch {
    // Storage may be unavailable; the purchase then only survives while the modal stays open.
  }
}

const sameCurrency = (a: string, b: string) => a.trim().toUpperCase() === b.trim().toUpperCase()

/**
 * Buys credit for the customer's own wallet with a saved payment method or a new card. The
 * customer reviews the price first; the charge is keyed so a double submit or a retry never charges
 * twice. A wallet without an id is the customer's money wallet, opened by the purchase.
 */
export const TopUpModal = ({
  wallet,
  paymentMethods,
  defaultPaymentMethodId,
  initialAmount,
  onClose,
}: {
  wallet: Wallet
  paymentMethods: CustomerPaymentMethod[]
  defaultPaymentMethodId?: string
  initialAmount?: bigint
  onClose: () => void
}) => {
  const invalidatePortal = useInvalidatePortal()
  const { overview } = usePortalNav()
  const [review, setReview] = useState<Review | null>(() => loadPending(wallet))
  const creditUnit = wallet.creditUnit
  const salePrice = wallet.salePrice
  const codec = walletAmountCodec(wallet)
  const schema = buildSchema(wallet.currency, creditUnit)
  const methods = useZodForm({
    schema,
    mode: 'onChange',
    defaultValues: {
      amount: codec.toInput(review ? review.amount : initialAmount),
      paymentMethodId:
        review?.paymentMethodId ?? defaultPaymentMethodId ?? paymentMethods[0]?.id ?? '',
    },
  })
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  // Once a charge was sent, its key is the only one: another purchase could charge twice.
  const [attempted, setAttempted] = useState(() => review !== null)
  // The charge is pending the customer's bank or the hosted checkout: the next click opens that
  // step again (pay() re-requests the same idempotency key and the server hands back its URL),
  // never resubmits the charge as if it were a plain retry.
  const [needsAuthentication, setNeedsAuthentication] = useState(false)
  const [checkout, setCheckout] = useState<Checkout | null>(null)
  const [error, setError] = useState<string>()
  const quoteMutation = useMutation(quoteTopUp)
  const topUpMutation = useMutation(topUpWallet)
  const statusMutation = useMutation(getTopUp)
  const balancesMutation = useMutation(getBalances)
  // Apart from `balancesMutation`: following the credit never blocks closing the modal.
  const landingMutation = useMutation(getBalances)
  const balanceBefore = useRef(wallet.available)
  const following = useRef(false)
  const persist = (walletId: string, pending: Review | null) =>
    savePending({ id: walletId, currency: wallet.currency, creditUnit: wallet.creditUnit }, pending)
  const busy =
    quoteMutation.isPending ||
    topUpMutation.isPending ||
    statusMutation.isPending ||
    balancesMutation.isPending

  // What the server reports a sent purchase became; true once it is settled either way.
  const applyState = async (
    pending: Review,
    state: GetTopUpResponse,
    failure?: string
  ): Promise<boolean> => {
    switch (state.status) {
      case TopUpStatus.SUCCEEDED:
      case TopUpStatus.PENDING:
        setOutcome(state.status === TopUpStatus.SUCCEEDED ? 'succeeded' : 'processing')
        setCheckout(null)
        persist(pending.walletId, null)
        await invalidatePortal()
        return true
      case TopUpStatus.REQUIRES_ACTION:
        setAttempted(true)
        setNeedsAuthentication(true)
        setError(
          failure ??
            (pending.paymentMethodId
              ? 'Your bank needs to confirm this payment.'
              : 'The payment is not complete yet.')
        )
        return false
      default:
        persist(pending.walletId, null)
        setReview(null)
        setAttempted(false)
        setNeedsAuthentication(false)
        setCheckout(null)
        setError(
          state.status === TopUpStatus.FAILED
            ? `${state.failureReason ?? 'The payment failed.'} You can try again or choose another payment method.`
            : (failure ?? 'The payment was not sent. Review your purchase again.')
        )
        return true
    }
  }

  // A purchase sent without a wallet opened the customer's money wallet: it is followed under
  // that wallet's id once the wallet exists.
  const adopt = async (pending: Review): Promise<Review> => {
    if (pending.walletId) return pending
    let opened
    try {
      const { wallets } = await balancesMutation.mutateAsync({})
      opened = wallets.find(
        candidate => !candidate.creditUnit && sameCurrency(candidate.currency, wallet.currency)
      )
    } catch {
      return pending
    }
    if (!opened) return pending
    const adopted = { ...pending, walletId: opened.id }
    persist(opened.id, adopted)
    setReview(adopted)
    return adopted
  }

  // The server decides what a sent purchase became; only an unknown outcome keeps its key.
  const reconcile = async (pending: Review, failure?: string) => {
    const sent = await adopt(pending)
    if (!sent.walletId) {
      if (failure) setError(failure)
      return
    }
    let state
    try {
      state = await statusMutation.mutateAsync({
        walletId: sent.walletId,
        idempotencyKey: sent.idempotencyKey,
      })
    } catch {
      if (failure) setError(failure)
      return
    }
    await applyState(sent, state, failure)
  }

  useEffect(() => {
    const pending = loadPending(wallet)
    if (pending) void reconcile(pending)
    // Once per opening, against the purchase stored when the modal was last closed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const latest = useRef({ applyState, adopt, check: statusMutation.mutateAsync })
  latest.current = { applyState, adopt, check: statusMutation.mutateAsync }

  // Refreshes the portal once the credit shows. Not cancelled on close: "Done" may come first.
  useEffect(() => {
    if (outcome !== 'succeeded' || following.current) return
    following.current = true
    const purchaseKey = review?.walletId || openingPurchaseKey(wallet.currency)
    const bought = review?.amount ?? 0n
    const fetchBalances = landingMutation.mutateAsync
    const startedAt = Date.now()
    const poll = async () => {
      try {
        const { wallets } = await fetchBalances({})
        if (topUpCreditLanded(wallets, purchaseKey, balanceBefore.current, bought)) {
          await invalidatePortal()
          return
        }
      } catch {
        // Retried on the next tick; the portal refreshes at the limit regardless.
      }
      if (Date.now() - startedAt > LANDING_POLL_LIMIT_MS) {
        await invalidatePortal()
        return
      }
      setTimeout(poll, LANDING_POLL_MS)
    }
    setTimeout(poll, LANDING_POLL_MS)
    // Once per settled purchase; the loop ends by itself.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [outcome])

  // The hosted checkout runs in another tab: follow the purchase there until it settles.
  useEffect(() => {
    if (!checkout || checkout.timedOut || !review) return
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const startedAt = Date.now()
    const poll = async () => {
      try {
        const sent = await latest.current.adopt(review)
        if (cancelled) return
        if (sent.walletId) {
          const state = await latest.current.check({
            walletId: sent.walletId,
            idempotencyKey: sent.idempotencyKey,
          })
          if (cancelled) return
          if (state.status !== TopUpStatus.REQUIRES_ACTION && state.status !== undefined) {
            if (await latest.current.applyState(sent, state)) return
          }
        }
      } catch {
        // A failed check is retried on the next tick; the purchase itself is unaffected.
      }
      if (cancelled) return
      if (Date.now() - startedAt > CHECKOUT_POLL_LIMIT_MS) {
        setCheckout(current => current && { ...current, timedOut: true })
        return
      }
      timer = setTimeout(poll, CHECKOUT_POLL_MS)
    }
    timer = setTimeout(poll, CHECKOUT_POLL_MS)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [checkout, review])

  const reviewPurchase = methods.handleSubmit(async values => {
    setError(undefined)
    const amount = codec.parse(values.amount) ?? 0n
    try {
      const response = await quoteMutation.mutateAsync({ walletId: wallet.id, amount })
      if (!response.quote) {
        setError('The price is unavailable. Nothing was charged.')
        return
      }
      setReview({
        walletId: wallet.id,
        amount,
        paymentMethodId: values.paymentMethodId,
        quote: response.quote,
        idempotencyKey: crypto.randomUUID(),
      })
    } catch (cause) {
      setError(customerError(cause, 'The price is unavailable. Nothing was charged.'))
    }
  })

  const pay = async () => {
    if (!review) return
    setError(undefined)
    setNeedsAuthentication(false)
    setAttempted(true)
    persist(review.walletId, review)
    const newCard = !review.paymentMethodId
    const embedded = newCard && isEmbeddedPortal()
    const purchaseKey = review.walletId || openingPurchaseKey(wallet.currency)
    // Opened within the click: a tab opened once the request returns is blocked as a popup.
    const tab = embedded ? window.open('', '_blank') : null
    try {
      const response = await topUpMutation.mutateAsync({
        walletId: review.walletId,
        amount: review.amount,
        paymentMethodId: review.paymentMethodId,
        idempotencyKey: review.idempotencyKey,
        expectedQuote: review.quote,
        // Embedded, the checkout opens in a tab with no portal session: this tab follows it.
        returnUrl:
          newCard && !embedded ? topUpReturnUrl(window.location.href, purchaseKey) : undefined,
      })
      if (response.checkoutUrl) {
        if (!embedded) {
          window.location.assign(response.checkoutUrl)
          return
        }
        const opened = !!tab && !tab.closed
        if (tab && opened) {
          tab.opener = null
          tab.location.href = response.checkoutUrl
        }
        setCheckout({ url: response.checkoutUrl, opened, timedOut: false })
        return
      }
      tab?.close()
      setOutcome(response.status === TopUpStatus.SUCCEEDED ? 'succeeded' : 'processing')
      persist(review.walletId, null)
      await invalidatePortal()
    } catch (cause) {
      tab?.close()
      await reconcile(review, customerError(cause, 'The payment outcome is not known yet.'))
    }
  }

  // An unpaid hosted checkout charges nothing until the customer pays it there, so it may be left
  // for a new purchase; a saved card's charge may still settle and keeps its key.
  const startOver = () => {
    if (review) persist(review.walletId, null)
    setReview(null)
    setAttempted(false)
    setNeedsAuthentication(false)
    setCheckout(null)
    setError(undefined)
  }

  const paymentMethod = paymentMethods.find(method => method.id === review?.paymentMethodId)
  // A unit wallet without a price in the customer's currency cannot be bought; a purchase
  // already sent is still followed through.
  const unsellable = !!creditUnit && !salePrice && !review
  const presets = creditUnit
    ? salePrice
      ? roundCreditPresets(salePrice.unitPrice, creditUnit.precision, salePrice.currency)
      : []
    : topUpPresets(wallet.currency)
  const amountInput = methods.watch('amount')
  const typedCredits = creditUnit ? codec.parse(amountInput) : null
  const estimate =
    creditUnit && salePrice && typedCredits !== null && typedCredits > 0n
      ? creditsToMoneyMinor(
          typedCredits,
          creditUnit.precision,
          salePrice.unitPrice,
          salePrice.currency
        )
      : null
  const added = creditUnit ? `The ${creditUnit.pluralName} are` : 'The credit is'
  const linkStyle = { color: 'var(--mtp-accent-ink)', fontSize: 13, fontWeight: 500 }

  return (
    <Modal open onClose={() => !busy && onClose()} maxWidth={440}>
      <div
        style={{
          padding: '20px 22px 16px',
          borderBottom: '1px solid var(--mtp-border)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
        }}
      >
        <span style={{ fontSize: 16, fontWeight: 600, letterSpacing: '-0.02em' }}>
          {creditUnit ? `Buy ${creditUnit.pluralName}` : 'Add credit'}
        </span>
        <ModalCloseButton onClose={() => !busy && onClose()} />
      </div>

      {outcome ? (
        <div style={{ padding: 22, display: 'flex', flexDirection: 'column', gap: 16 }}>
          <p style={{ fontSize: 13, margin: 0 }}>
            {outcome === 'succeeded'
              ? `Payment of ${formatCurrency(review!.quote.charge, review!.quote.currency)} accepted. Your balance updates in a moment.`
              : `Payment of ${formatCurrency(review!.quote.charge, review!.quote.currency)} is processing. ${added} added once it settles.`}
          </p>
          <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
            <PButton
              onClick={() => {
                void invalidatePortal()
                onClose()
              }}
            >
              Done
            </PButton>
          </div>
        </div>
      ) : checkout && review ? (
        <div style={{ padding: 22, display: 'flex', flexDirection: 'column', gap: 12 }}>
          <p style={{ fontSize: 13, margin: 0 }}>
            {checkout.opened
              ? `Pay ${formatCurrency(review.quote.charge, review.quote.currency)} on the secure payment page that opened in a new tab.`
              : `Your browser blocked the new tab. Open the secure payment page to pay ${formatCurrency(review.quote.charge, review.quote.currency)}.`}
          </p>
          <a href={checkout.url} target="_blank" rel="noopener noreferrer" style={linkStyle}>
            {checkout.opened ? 'Open the payment page again' : 'Open the payment page'}
          </a>
          <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
            {checkout.timedOut
              ? 'We stopped checking for the payment. Check again once you have paid.'
              : 'This window updates once the payment goes through.'}
          </span>
          {error && (
            <span role="alert" style={errorStyle}>
              {error}
            </span>
          )}
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 10 }}>
            <PButton variant="secondary" onClick={onClose} disabled={busy}>
              Close
            </PButton>
            {checkout.timedOut && (
              <PButton
                onClick={() => setCheckout({ ...checkout, timedOut: false })}
                loading={statusMutation.isPending}
              >
                Check again
              </PButton>
            )}
          </div>
        </div>
      ) : unsellable ? (
        <div style={{ padding: 22, display: 'flex', flexDirection: 'column', gap: 16 }}>
          <p style={{ fontSize: 13, margin: 0 }}>
            {creditUnit?.pluralName} cannot be bought here at the moment. Contact{' '}
            {contactName(overview.invoicingEntityName)} to add more.
          </p>
          <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
            <PButton onClick={onClose}>Close</PButton>
          </div>
        </div>
      ) : (
        <form onSubmit={reviewPurchase}>
          <div style={{ padding: '20px 22px', display: 'flex', flexDirection: 'column', gap: 18 }}>
            <p style={{ fontSize: 12.5, color: 'var(--mtp-text-2)', margin: 0 }}>
              {`Your payment method is charged now and ${topUpSettlesCopy(creditUnit)} Your bank may ask you to confirm the payment.`}
            </p>
            <>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
                <span style={labelStyle}>
                  {creditUnit
                    ? `Quantity (${creditUnit.pluralName})`
                    : `Amount (${wallet.currency})`}
                </span>
                <input
                  style={fieldStyle}
                  inputMode="decimal"
                  placeholder={creditUnit ? '0' : '0.00'}
                  autoFocus
                  disabled={!!review}
                  {...methods.register('amount')}
                />
                {methods.formState.errors.amount && (
                  <span style={errorStyle}>{methods.formState.errors.amount.message}</span>
                )}
                {!review && (
                  <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                    {presets.map(amount => (
                      <button
                        key={amount.toString()}
                        type="button"
                        className="mtp-btn mtp-btn-secondary"
                        onClick={() =>
                          methods.setValue('amount', codec.toInput(amount), {
                            shouldValidate: true,
                            shouldDirty: true,
                          })
                        }
                        style={{
                          padding: '5px 10px',
                          fontSize: 12.5,
                          fontWeight: 500,
                          fontFamily: 'inherit',
                          cursor: 'pointer',
                          borderRadius: 6,
                          border: '1px solid var(--mtp-border)',
                          background: 'transparent',
                          color: 'var(--mtp-text-2)',
                        }}
                      >
                        {creditUnit
                          ? formatCreditQuantity(amount, creditUnit.precision)
                          : formatCurrency(amount, wallet.currency)}
                      </button>
                    ))}
                  </div>
                )}
                {!review &&
                  creditUnit &&
                  salePrice &&
                  typedCredits !== null &&
                  estimate !== null && (
                    <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
                      Buy {formatCredits(typedCredits, creditUnit)} at your price of{' '}
                      {formatCurrencyNoRounding(salePrice.unitPrice, salePrice.currency)} per{' '}
                      {creditUnit.name} — {formatCurrency(estimate, salePrice.currency)} + tax
                    </span>
                  )}
              </label>
              <label style={{ display: 'flex', flexDirection: 'column', gap: 7 }}>
                <span style={labelStyle}>Payment method</span>
                <select
                  style={fieldStyle}
                  disabled={!!review}
                  {...methods.register('paymentMethodId')}
                >
                  {paymentMethods.map(method => (
                    <option key={method.id} value={method.id}>
                      {pmLabel(method)}
                    </option>
                  ))}
                  <option value="">Use a new card</option>
                </select>
              </label>
            </>
            {review && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 4, fontSize: 13 }}>
                {creditUnit ? (
                  <span style={{ color: 'var(--mtp-text-2)' }}>
                    {formatCredits(review.amount, creditUnit)}
                    {salePrice
                      ? ` at your price of ${formatCurrencyNoRounding(salePrice.unitPrice, salePrice.currency)} per ${creditUnit.name}`
                      : ''}
                    : {formatCurrency(review.quote.net, review.quote.currency)}
                    {review.quote.tax > 0n
                      ? ` + tax ${formatCurrency(review.quote.tax, review.quote.currency)}`
                      : ''}
                  </span>
                ) : (
                  review.quote.tax > 0n && (
                    <span style={{ color: 'var(--mtp-text-2)' }}>
                      Credit {formatCurrency(review.quote.net, review.quote.currency)} + tax{' '}
                      {formatCurrency(review.quote.tax, review.quote.currency)}
                    </span>
                  )
                )}
                <span style={{ fontWeight: 600 }}>
                  You pay {formatCurrency(review.quote.charge, review.quote.currency)}
                  {paymentMethod
                    ? ` with ${pmLabel(paymentMethod)}`
                    : review.paymentMethodId
                      ? ''
                      : ' with a new card on a secure payment page'}
                  .
                </span>
              </div>
            )}
            {error && (
              <span role="alert" style={errorStyle}>
                {error}
              </span>
            )}
          </div>
          <div
            style={{
              padding: '14px 22px 20px',
              display: 'flex',
              justifyContent: 'flex-end',
              gap: 10,
            }}
          >
            {review ? (
              <>
                {!attempted ? (
                  <PButton variant="secondary" onClick={() => setReview(null)} disabled={busy}>
                    Change
                  </PButton>
                ) : (
                  !review.paymentMethodId && (
                    <PButton variant="secondary" onClick={startOver} disabled={busy}>
                      Start over
                    </PButton>
                  )
                )}
                <PButton
                  onClick={pay}
                  loading={topUpMutation.isPending || statusMutation.isPending}
                >
                  {needsAuthentication
                    ? review.paymentMethodId
                      ? 'Continue to your bank'
                      : 'Continue to payment'
                    : attempted
                      ? 'Retry this payment'
                      : 'Confirm and pay'}
                </PButton>
              </>
            ) : (
              <>
                <PButton variant="secondary" onClick={onClose} disabled={busy}>
                  Cancel
                </PButton>
                <PButton
                  type="submit"
                  loading={quoteMutation.isPending || balancesMutation.isPending}
                >
                  Review
                </PButton>
              </>
            )}
          </div>
        </form>
      )}
    </Modal>
  )
}
