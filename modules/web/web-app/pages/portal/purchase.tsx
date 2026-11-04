import { Code, ConnectError } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { Form, cn } from '@md/ui'
import { AlertCircle, Loader2, Server } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { useParams, useSearchParams } from 'react-router-dom'
import { z } from 'zod'

import { CheckoutThemePane } from '@/features/checkout/CheckoutThemePane'
import { resolveCheckoutTheme } from '@/features/checkout/resolveCheckoutTheme'
import { BillingInfoFields } from '@/features/customers/components/BillingInfoForm'
import { solveAltcha } from '@/features/purchase/pow'
import {
  formatSubscriptionFee,
  formatSubscriptionFeeBillingPeriod,
} from '@/features/subscriptions/utils/fees'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  getPurchaseOffer,
  startPurchase,
} from '@/rpc/portal/purchase/v1/purchase-PurchaseService_connectquery'
import { BuyerType, TaxIdCollection } from '@/rpc/portal/purchase/v1/purchase_pb'
import { useForceTheme } from 'providers/ThemeProvider'

import type { PurchaseOffer } from '@/rpc/portal/purchase/v1/purchase_pb'

const optionalText = (max: number) => z.string().trim().max(max).optional()

const purchaseSchema = (
  requireAddress: boolean,
  requirePostalCode: boolean,
  requireTaxId: boolean
) =>
  z
    .object({
      billingEmail: z.string().trim().email('Enter a valid email address').max(254),
      buyerType: z.enum(['COMPANY', 'INDIVIDUAL']),
      name: optionalText(200),
      firstName: optionalText(200),
      lastName: optionalText(200),
      country: z.string().min(2, 'Select a country'),
      line1: optionalText(200),
      line2: optionalText(200),
      city: optionalText(200),
      state: optionalText(200),
      zipCode: optionalText(200),
      vatNumber: optionalText(64),
      legalNumber: optionalText(64),
    })
    .superRefine((values, ctx) => {
      const required: [keyof typeof values, string][] =
        values.buyerType === 'COMPANY'
          ? [['name', 'Legal name is required']]
          : [
              ['firstName', 'First name is required'],
              ['lastName', 'Last name is required'],
            ]
      if (requireTaxId && values.buyerType === 'COMPANY') {
        required.push(['vatNumber', 'Tax number is required'])
      }
      if (requireAddress) {
        required.push(['line1', 'Address is required'], ['city', 'City is required'])
      }
      if (requirePostalCode) {
        required.push(['zipCode', 'Postal code is required'])
      }
      for (const [field, message] of required) {
        if (!values[field]?.trim()) ctx.addIssue({ code: 'custom', path: [field], message })
      }
    })

type PurchaseValues = z.infer<ReturnType<typeof purchaseSchema>>

const buyerTypeLabels = {
  COMPANY: 'Company',
  INDIVIDUAL: 'Individual',
} as const

const blankToUndefined = (value?: string) => value?.trim() || undefined

const SELLER_UNAVAILABLE =
  "This plan can't be purchased right now because of an issue on the seller's side. Please contact the seller."

const startPurchaseErrorMessage = (error: unknown): string => {
  if (error instanceof ConnectError) {
    switch (error.code) {
      case Code.ResourceExhausted:
        return 'Too many attempts. Please wait a moment and try again.'
      case Code.InvalidArgument:
        return error.rawMessage || 'Please check the information you entered.'
      case Code.PermissionDenied:
      case Code.Unauthenticated:
        return 'We could not verify your request. Please try again.'
      case Code.NotFound:
        return 'This purchase link is no longer available.'
      case Code.FailedPrecondition:
      case Code.Unavailable:
        return SELLER_UNAVAILABLE
    }
  }
  return 'Something went wrong. Please try again in a moment.'
}

const CenteredState = ({ title, message }: { title: string; message: string }) => (
  <div className="min-h-screen w-full bg-[#00000002] flex flex-col items-center justify-center px-6 py-12 text-center">
    <AlertCircle className="h-8 w-8 text-muted-foreground mb-4" />
    <h2 className="text-md font-semibold text-gray-800 mb-2">{title}</h2>
    <p className="text-gray-800 text-sm max-w-md">{message}</p>
  </div>
)

const attemptIdFor = (token: string): string => {
  const key = `purchase-attempt:${token}`
  try {
    const existing = sessionStorage.getItem(key)
    if (existing) return existing
    const id = crypto.randomUUID()
    sessionStorage.setItem(key, id)
    return id
  } catch {
    return crypto.randomUUID()
  }
}

/** Solves the offer's proof-of-work challenge in the background, as soon as it's known. */
const useProofOfWork = (challenge: string) => {
  const [solution, setSolution] = useState<string | null>(null)
  const pending = useRef<Promise<string> | null>(null)

  useEffect(() => {
    setSolution(null)
    if (!challenge) {
      pending.current = Promise.resolve('')
      return
    }
    const controller = new AbortController()
    const promise = solveAltcha(challenge, controller.signal)
    pending.current = promise
    promise.then(setSolution, () => {})
    return () => controller.abort()
  }, [challenge])

  const wait = useCallback(() => pending.current ?? Promise.resolve(''), [])
  return { solution, wait }
}

const taxNote = (offer: PurchaseOffer) =>
  offer.taxInclusive
    ? `All prices incl. tax.${offer.allowCouponCodes ? ' Promotion codes are applied at the next step.' : ''}`
    : `All prices excl. tax. ${offer.allowCouponCodes ? 'Tax and promotion codes are' : 'Tax is'} applied at the next step.`

const OfferSummary = ({ offer }: { offer: PurchaseOffer }) => (
  <div className="text-sm">
    <div className="items-center mb-6 flex">
      {offer.sellerLogoUrl && (
        <img
          src={offer.sellerLogoUrl}
          alt={`${offer.sellerName} logo`}
          className="h-7 w-auto mr-1"
        />
      )}
      <span className="font-semibold text-md">{offer.sellerName}</span>
    </div>

    <div className="mb-6">
      <h1 className="text-base font-normal mb-1 text-muted-foreground">
        Subscribe to {offer.planName}
      </h1>
      {offer.trialDays ? (
        <div className="mt-2 inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-blue-100 text-blue-800">
          {offer.trialDays} day trial
        </div>
      ) : null}
      {offer.planDescription && (
        <p className="mt-3 text-muted-foreground whitespace-pre-line">{offer.planDescription}</p>
      )}
    </div>

    <div className="py-4 space-y-4">
      {offer.components.map((component, i) => {
        const pricing = formatSubscriptionFee(component.fee, offer.currency)
        const sublabel = [
          pricing.details && !pricing.redundantDetails ? pricing.details : undefined,
          formatSubscriptionFeeBillingPeriod(component.period),
        ]
          .filter(Boolean)
          .join(' · ')
        return (
          <div key={`${component.name}-${i}`} className="flex justify-between gap-4">
            <div>
              <div className="font-medium">{component.name}</div>
              <div className="text-xs text-muted-foreground">{sublabel}</div>
              {pricing.breakdown && (
                <pre className="mt-0.5 text-[11px] text-muted-foreground font-mono whitespace-pre leading-tight">
                  {pricing.breakdown}
                </pre>
              )}
            </div>
            <div className="font-medium text-right">{pricing.amount}</div>
          </div>
        )
      })}
    </div>

    <div className="border-t border-gray-200 py-4 text-xs text-muted-foreground">
      {taxNote(offer)}
    </div>

    {offer.forInstallation && (
      <div className="mt-2 flex items-start gap-3 rounded-lg border border-border bg-background p-4 text-muted-foreground">
        <Server className="h-4 w-4 mt-0.5 shrink-0" />
        <p>
          The license goes to installation{' '}
          <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs text-foreground break-all">
            {offer.installationId ?? 'unknown'}
          </code>{' '}
          once your subscription is active: check it is the one your License settings show.
        </p>
      </div>
    )}
  </div>
)

const PurchaseForm = ({
  offer,
  token,
  licenseRequest,
  onChallengeSpent,
}: {
  offer: PurchaseOffer
  token: string
  licenseRequest?: string
  onChallengeSpent: () => void
}) => {
  const [attemptId] = useState(() => attemptIdFor(token))
  const pow = useProofOfWork(offer.powChallenge)
  const [verifying, setVerifying] = useState(false)
  const [submitError, setSubmitError] = useState<string | null>(null)
  const [redirecting, setRedirecting] = useState(false)

  const buyerTypes = (offer.buyerTypes.length ? offer.buyerTypes : [BuyerType.COMPANY]).map(t =>
    t === BuyerType.INDIVIDUAL ? 'INDIVIDUAL' : 'COMPANY'
  )
  const taxNumber = (
    {
      [TaxIdCollection.OFF]: 'hidden',
      [TaxIdCollection.OPTIONAL]: 'optional',
      [TaxIdCollection.REQUIRED]: 'required',
    } as const
  )[offer.taxIdCollection]
  const requireAddress = offer.requireBillingAddress || offer.requireBillingInformation
  const schema = useMemo(
    () => purchaseSchema(requireAddress, offer.requireBillingInformation, taxNumber === 'required'),
    [requireAddress, offer.requireBillingInformation, taxNumber]
  )
  const methods = useZodForm({
    schema,
    defaultValues: {
      billingEmail: '',
      buyerType: buyerTypes[0],
      name: '',
      firstName: '',
      lastName: '',
      country: '',
      line1: '',
      line2: '',
      city: '',
      state: '',
      zipCode: '',
      vatNumber: '',
      legalNumber: '',
    },
    mode: 'onSubmit',
    reValidateMode: 'onChange',
  })
  const buyerType = useWatch({ control: methods.control, name: 'buyerType' })
  const isCompany = buyerType === 'COMPANY'

  const start = useMutation(startPurchase)

  const onSubmit = async (values: PurchaseValues) => {
    setSubmitError(null)
    try {
      setVerifying(true)
      const powSolution = await pow.wait().finally(() => setVerifying(false))
      const res = await start.mutateAsync({
        token,
        email: values.billingEmail.trim(),
        buyerType: isCompany ? BuyerType.COMPANY : BuyerType.INDIVIDUAL,
        companyName: isCompany ? values.name?.trim() : undefined,
        firstName: isCompany ? undefined : values.firstName?.trim(),
        lastName: isCompany ? undefined : values.lastName?.trim(),
        country: values.country,
        billingAddress: requireAddress
          ? {
              line1: blankToUndefined(values.line1),
              line2: blankToUndefined(values.line2),
              city: blankToUndefined(values.city),
              state: blankToUndefined(values.state),
              zipCode: blankToUndefined(values.zipCode),
            }
          : undefined,
        vatNumber:
          isCompany && taxNumber !== 'hidden' ? blankToUndefined(values.vatNumber) : undefined,
        legalNumber: isCompany ? blankToUndefined(values.legalNumber) : undefined,
        powSolution,
        attemptId,
        licenseRequest,
      })
      setRedirecting(true)
      window.location.href = res.checkoutUrl
    } catch (error) {
      setSubmitError(startPurchaseErrorMessage(error))
      onChallengeSpent()
    }
  }

  const busy = verifying || start.isPending || redirecting

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4" noValidate>
        <h2 className="text-base font-medium" style={{ color: 'var(--mtp-text)' }}>
          Billing information
        </h2>

        {buyerTypes.length > 1 && (
          <div
            role="radiogroup"
            aria-label="Buying as"
            className="grid grid-cols-2 gap-1 p-1 rounded-lg"
            style={{ background: 'var(--mtp-track)' }}
          >
            {buyerTypes.map(value => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={buyerType === value}
                onClick={() => methods.setValue('buyerType', value)}
                className={cn(
                  'rounded-md py-1.5 text-xs font-medium transition-colors',
                  buyerType === value ? 'shadow-sm' : 'opacity-70 hover:opacity-100'
                )}
                style={{
                  background: buyerType === value ? 'var(--mtp-surface)' : 'transparent',
                  color: 'var(--mtp-text)',
                }}
              >
                {buyerTypeLabels[value]}
              </button>
            ))}
          </div>
        )}

        <BillingInfoFields
          individual={!isCompany}
          address={requireAddress ? 'required' : 'country'}
          taxNumber={taxNumber}
          autofill
        />

        {submitError && (
          <div
            role="alert"
            className="p-3 rounded-lg text-sm flex items-start"
            style={{ background: 'var(--mtp-danger-bg)', color: 'var(--mtp-danger)' }}
          >
            <AlertCircle size={16} className="mr-2 mt-0.5 shrink-0" />
            <span>{submitError}</span>
          </div>
        )}

        <button
          type="submit"
          disabled={busy}
          className="w-full py-3 rounded-lg transition-all font-medium disabled:cursor-not-allowed disabled:opacity-60 hover:opacity-90"
          style={{ background: 'var(--mtp-accent)', color: 'var(--mtp-on-accent)' }}
        >
          {busy ? (
            <span className="flex items-center justify-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              {verifying ? 'Verifying…' : redirecting ? 'Redirecting to checkout…' : 'Please wait…'}
            </span>
          ) : (
            'Continue to payment'
          )}
        </button>

        <p className="text-xs" style={{ color: 'var(--mtp-text-3)' }}>
          You will review the order and enter payment details on the next page.
        </p>
      </form>
    </Form>
  )
}

export const PortalPurchase = () => {
  useForceTheme('light')
  const { token = '' } = useParams()
  const [searchParams] = useSearchParams()
  const licenseRequest = searchParams.get('license_request') ?? undefined

  const offerQuery = useQuery(
    getPurchaseOffer,
    { token, licenseRequest },
    { enabled: !!token, retry: false, refetchOnWindowFocus: false }
  )
  const offer = offerQuery.data?.offer

  const themeConfig = useMemo(
    () =>
      resolveCheckoutTheme({
        brandColor: offer?.sellerBrandColor,
        logoUrl: offer?.sellerLogoUrl,
        tradeName: offer?.sellerName,
      }),
    [offer]
  )

  if (offerQuery.error && !offer) {
    const code = offerQuery.error.code
    if (code === Code.FailedPrecondition || code === Code.Unavailable) {
      return <CenteredState title="Purchase unavailable" message={SELLER_UNAVAILABLE} />
    }
    if (code === Code.NotFound || code === Code.InvalidArgument) {
      return (
        <CenteredState
          title="This link is no longer available"
          message="The purchase link is invalid or has been disabled. Please ask the seller for an up-to-date link."
        />
      )
    }
    if (code === Code.ResourceExhausted) {
      return (
        <CenteredState
          title="Too many requests"
          message="Please wait a moment, then reload this page."
        />
      )
    }
    return (
      <CenteredState
        title="Something went wrong"
        message="There may be a connection issue. Please reload this page in a moment."
      />
    )
  }

  if (!offer) {
    return (
      <div className="min-h-screen w-full flex items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="w-full flex flex-col lg:flex-row min-h-screen lg:h-screen lg:overflow-hidden">
      <div className="flex flex-col bg-background-gray gap-5 w-full lg:w-3/5 px-5 py-6 lg:px-10 lg:py-12 xl:px-20 xl:pt-16 xl:pb-20 border-b lg:border-b-0 lg:border-r border-border-regular lg:h-screen lg:overflow-auto">
        <div className="w-full max-w-[500px] mx-auto">
          <OfferSummary offer={offer} />
        </div>
      </div>
      <CheckoutThemePane
        config={themeConfig}
        className="w-full lg:w-2/5 flex flex-col px-5 py-6 lg:px-10 lg:py-12 xl:px-20 xl:pt-16 lg:h-screen lg:overflow-auto shadow-md"
      >
        <div className="w-full max-w-[440px] mx-auto lg:mx-0">
          <PurchaseForm
            offer={offer}
            token={token}
            licenseRequest={licenseRequest}
            onChallengeSpent={() => void offerQuery.refetch()}
          />
        </div>
      </CheckoutThemePane>
    </div>
  )
}
