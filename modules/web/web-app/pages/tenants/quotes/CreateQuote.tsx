import { create } from '@bufbuild/protobuf'
import { Code, ConnectError } from '@connectrpc/connect'
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { Alert, Button, Form } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { addDays, startOfDay } from 'date-fns'
import { ArrowLeft, Save } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router'
import { toast } from 'sonner'

import { Slot, hasSlot } from '@/edition'
import { scopeOptionsWithPending } from '@/features/billing/minimumCommitment'
import { resolveEntitlementSpecs } from '@/features/entitlements/creation/resolveEntitlementSpecs'
import {
  DraftConflictBanner,
  DraftRestoreBanner,
  StaleQuoteBanner,
} from '@/features/quotes/editor/AutosaveBanners'
import { DocumentSection } from '@/features/quotes/editor/DocumentSection'
import { EntitlementsBlock } from '@/features/quotes/editor/EntitlementsBlock'
import { InternalSection } from '@/features/quotes/editor/InternalSection'
import { LinesSection } from '@/features/quotes/editor/LinesSection'
import { PeopleSection } from '@/features/quotes/editor/PeopleSection'
import { QuoteEditorHeader } from '@/features/quotes/editor/QuoteEditorHeader'
import { QuoteEditorRail, QuoteFullPreview } from '@/features/quotes/editor/QuoteEditorRail'
import { QuoteEditorOutline } from '@/features/quotes/editor/QuoteSectionNav'
import { TermsSection } from '@/features/quotes/editor/TermsSection'
import { buildCreateQuote, previewFormValues } from '@/features/quotes/editor/buildCreateQuote'
import { fromTemplatePayload, toTemplatePayload } from '@/features/quotes/editor/draftPayload'
import { createQuoteSchema } from '@/features/quotes/editor/quoteFormSchema'
import { buildPreviewQuote, useQuotePreviewLines } from '@/features/quotes/editor/quotePreview'
import {
  discountError,
  editorLines,
  newQuoteV2State,
  orderLines,
  v2Presence,
} from '@/features/quotes/editor/quoteV2'
import { useQuoteAutosave } from '@/features/quotes/editor/useQuoteAutosave'
import { useQuoteV2Gates } from '@/features/quotes/editor/useQuoteV2Gates'
import {
  QuoteFormSeed,
  emptyPriceComponentsState,
  generateQuoteNumber,
} from '@/features/quotes/quoteFormSeed'
import {
  PriceComponentsState,
  pricingConfigurationErrors,
} from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import { useBasePath } from '@/hooks/useBasePath'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { defaultNetTerms } from '@/lib/mapping/priceToSubscriptionFee'
import { effectiveSubscriptionPrices } from '@/pages/tenants/subscription/create/state'
import { listEntityActivity } from '@/rpc/api/activity/v1/activity-ActivityService_connectquery'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { listCreditUnits } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { createFeature } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  getInvoicingEntity,
  getInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { MinimumCommitment, PlanType } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanWithVersionByVersionId,
  listPlans,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import {
  createQuote,
  getQuote,
  listQuotes,
  updateQuote,
} from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { CreateQuoteRequestSchema, UpdateQuoteRequestSchema } from '@/rpc/api/quotes/v1/quotes_pb'

import type { PendingEntitlementSpec } from '@/features/entitlements/creation/types'
import type { EditorCrumb } from '@/features/quotes/editor/QuoteEditorHeader'
import type { SectionIssue } from '@/features/quotes/editor/editorSections'
import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { QuoteV2State } from '@/features/quotes/editor/quoteV2'
import type { QuoteDraft } from '@/rpc/api/quotes/v1/models_pb'
import type { ReactNode } from 'react'

/** The editor working on a template: no deal fields, no drafts, saved by the template's page. */
export interface QuoteTemplateEditing {
  crumbs: EditorCrumb[]
  /** The field editing the template's name. */
  title: ReactNode
  /** The template's own fields, under the name. */
  details: ReactNode
  canSave: boolean
  isSaving: boolean
  /** Receives the editor state as a template payload, as "Save as template" encodes it. */
  onSave: (payload: unknown) => void
}

interface CreateQuoteProps {
  seed?: QuoteFormSeed
  editQuoteId?: string
  /** The edited quote's `updatedAt` when loaded ('' when never updated). */
  loadedUpdatedAt?: string
  /** The edited quote's status; a new quote is a draft. */
  status?: QuoteStatus
  template?: { sourceQuoteNumber: string; planOutdated: boolean }
  /** The autosaved draft this new quote resumes. */
  initialDraft?: QuoteDraft
  templateEditing?: QuoteTemplateEditing
}

export const CreateQuote = ({
  seed,
  editQuoteId,
  loadedUpdatedAt,
  status = QuoteStatus.DRAFT,
  template,
  initialDraft,
  templateEditing,
}: CreateQuoteProps = {}) => {
  const isTemplate = Boolean(templateEditing)
  const navigate = useNavigate()
  const basePath = useBasePath()
  const queryClient = useQueryClient()
  const isEdit = Boolean(editQuoteId)
  const [baselineUpdatedAt] = useState(loadedUpdatedAt)
  const [staleSave, setStaleSave] = useState(false)
  const forceSave = useRef(false)
  const [priceComponentsState, setPriceComponentsState] = useState<PriceComponentsState>(
    () => seed?.priceComponents ?? emptyPriceComponentsState()
  )

  const [selectedAddOns, setSelectedAddOns] = useState<{ addOnId: string; quantity?: number }[]>(
    () => seed?.addOns ?? []
  )
  const [selectedCoupons, setSelectedCoupons] = useState<{ couponId: string }[]>(
    () => seed?.coupons ?? []
  )
  const [pendingEntitlements, setPendingEntitlements] = useState<PendingEntitlementSpec[]>(
    () => seed?.pendingEntitlements ?? []
  )
  const [v2, setV2] = useState<QuoteV2State>(() => seed?.v2 ?? newQuoteV2State())
  const [v2Present, setV2Present] = useState(() => v2Presence(v2))
  const v2Gates = useQuoteV2Gates(v2Present)

  // Collect product ids from in-flight (not-yet-persisted) extra and overridden price components.
  // Only components with an existing productId contribute — new-product refs have no id yet.
  const extraProductIds = useMemo(() => {
    const ids = new Set<string>()
    for (const c of priceComponentsState.components.extra) {
      if (c.productId) ids.add(c.productId)
    }
    for (const c of priceComponentsState.components.overridden) {
      if (c.productId) ids.add(c.productId)
    }
    return Array.from(ids)
  }, [priceComponentsState.components.extra, priceComponentsState.components.overridden])

  const createFeatureMutation = useMutation(createFeature)

  // Quote-level minimum override, applied at creation (bundled into the CreateQuote request).
  // Undefined = inherit the plan's term. Without the commitments module a new quote cannot carry
  // one (the server refuses it), while an edited quote keeps its own unchanged.
  const [minimumOverride, setMinimumOverride] = useState<MinimumCommitment | undefined>(() =>
    isEdit || hasSlot('quoteCreate.minimumCommitment') ? seed?.minimumCommitment : undefined
  )

  const createQuoteMutation = useMutation(createQuote, {
    onError: error => {
      toast.error(`Failed to create quote: ${error.message}`)
    },
  })

  const updateQuoteMutation = useMutation(updateQuote, {
    onSuccess: async response => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: getQuote.parent, cardinality: undefined }),
      })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listQuotes.parent, cardinality: undefined }),
      })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listEntityActivity, cardinality: undefined }),
      })
      if (response.quote?.quote?.status === QuoteStatus.PENDING) {
        toast.success(
          'Quote updated: its link shows the changes. Resend it to let the customer know.'
        )
      }
    },
    onError: error => {
      if (ConnectError.from(error).code !== Code.Aborted) {
        toast.error(`Failed to update quote: ${error.message}`)
      }
    },
  })

  const isSaving = createQuoteMutation.isPending || updateQuoteMutation.isPending
  const submitLabel = isEdit
    ? isSaving
      ? 'Saving…'
      : 'Save changes'
    : isSaving
      ? 'Creating…'
      : 'Create quote'

  const [searchParams, setSearchParams] = useSearchParams()

  const methods = useZodForm({
    schema: createQuoteSchema,
    defaultValues: seed?.values ?? {
      quote_number: generateQuoteNumber(),
      customer_id: searchParams.get('customerId')?.trim() ?? '',
      plan_version_id: '',
      recipients: [{ name: '', email: '' }],
      activation_condition: 'ON_START',
      payment_methods_type: 'online',
      auto_advance_invoices: true,
      charge_automatically: true,
      create_subscription_on_acceptance: false,
    },
  })

  const multipleSigners = useEntitlement(entitlementCodes.QUOTES_FULL)
  const [customerId, planVersionId, quoteNumber] = methods.watch([
    'customer_id',
    'plan_version_id',
    'quote_number',
  ])

  // Component customizations reference the plan's components, so they don't survive a plan change.
  const [pricingPlanVersionId, setPricingPlanVersionId] = useState(planVersionId)
  if (pricingPlanVersionId !== planVersionId) {
    setPricingPlanVersionId(planVersionId)
    setPriceComponentsState(emptyPriceComponentsState())
  }

  const customerQuery = useQuery(
    getCustomerById,
    { id: customerId ?? '' },
    { enabled: Boolean(customerId) }
  )

  useEffect(() => {
    if (customerQuery.data?.customer?.billingEmail) {
      const existing = methods.getValues('recipients')

      if (existing.length === 1 && existing[0].email === '') {
        methods.setValue('recipients', [
          {
            name: customerQuery.data.customer.name,
            email: customerQuery.data.customer.billingEmail,
          },
        ])
      }
    }
  }, [customerQuery.data?.customer])

  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: planVersionId ?? '' },
    { enabled: Boolean(planVersionId) }
  )

  // Fetch all plans for resolving trialing plan name
  const plansQuery = useQuery(listPlans, {})

  const planCurrency = planQuery.data?.plan?.version?.currency
  const planTrialConfig = planQuery.data?.plan?.version?.trialConfig
  const isFreePlan = planQuery.data?.plan?.plan?.planType === PlanType.FREE
  const planDefaultMinimum = planQuery.data?.plan?.version?.minimumCommitment

  const trialingPlanName = planTrialConfig?.trialingPlanId
    ? plansQuery.data?.plans.find(p => p.id === planTrialConfig.trialingPlanId)?.name
    : undefined

  const invoicingEntityQuery = useQuery(
    getInvoicingEntity,
    {
      id: customerQuery.data?.customer?.invoicingEntityId || '',
    },
    { enabled: Boolean(customerQuery.data?.customer?.invoicingEntityId) }
  )

  const creditUnitsQuery = useQuery(listCreditUnits, { includeArchived: true })
  const creditUnits = creditUnitsQuery.data?.units

  const priceComponentsQuery = useQuery(
    listPriceComponents,
    {
      planVersionId: planVersionId ?? '',
    },
    { enabled: Boolean(planVersionId) }
  )
  const planComponents = priceComponentsQuery.data?.components

  const removedProductIds = useMemo(() => {
    const removedSet = new Set(priceComponentsState.components.removed)
    return (planComponents ?? [])
      .filter(c => removedSet.has(c.id) && c.productId)
      .map(c => c.productId!)
  }, [priceComponentsState.components.removed, planComponents])

  const addOnsQuery = useQuery(
    listAddOns,
    planVersionId
      ? {
          planVersionId,
          pagination: {
            perPage: 100,
            page: 0,
          },
        }
      : skipToken
  )

  const couponsQuery = useQuery(listCoupons, {
    pagination: {
      perPage: 100,
      page: 0,
    },
    filter: ListCouponRequest_CouponFilter.ACTIVE,
  })

  const availableAddOns = addOnsQuery.data?.addOns || []
  const availableCoupons = couponsQuery.data?.coupons || []

  // Display only: unset terms are left to the server, which applies this same default.
  const planNetTerms = planQuery.data?.plan?.version?.netTerms
  const netTermsDefault =
    planCurrency && planNetTerms !== undefined && planComponents && addOnsQuery.data
      ? defaultNetTerms(
          effectiveSubscriptionPrices(
            planComponents,
            priceComponentsState.components,
            selectedAddOns.map(sel => availableAddOns.find(a => a.id === sel.addOnId)?.price),
            planCurrency
          ),
          planNetTerms
        )
      : undefined

  const invoicingEntityId = customerQuery.data?.customer?.invoicingEntityId
  const providersQuery = useQuery(
    getInvoicingEntityProviders,
    invoicingEntityId ? { id: invoicingEntityId } : skipToken
  )

  // A template has no customer, hence no invoicing entity to check: each quote checks its own.
  const hasOnlinePaymentProvider =
    isTemplate || !!providersQuery.data?.cardProvider || !!providersQuery.data?.directDebitProvider
  const isLoadingProviders = customerQuery.isLoading || providersQuery.isLoading

  const [activationCondition, paymentMethodsType, chargeAutomatically] = methods.watch([
    'activation_condition',
    'payment_methods_type',
    'charge_automatically',
  ])

  const { getValues, setValue, watch } = methods

  const orderedLines = orderLines(
    editorLines(planComponents ?? [], priceComponentsState, selectedAddOns, availableAddOns),
    v2.lineOrder
  )

  const pricingErrors = planComponents
    ? pricingConfigurationErrors(planComponents, priceComponentsState)
    : []
  const pricingValid = Boolean(planComponents) && pricingErrors.length === 0
  const hasDiscountError = orderedLines.some(l =>
    discountError(v2.lineDetails[l.key]?.discountPercent ?? '')
  )

  const getSnapshot = (): QuoteFormSeed => ({
    values: getValues(),
    priceComponents: priceComponentsState,
    addOns: selectedAddOns,
    coupons: selectedCoupons,
    minimumCommitment: minimumOverride,
    pendingEntitlements,
    v2,
  })

  const autosave = useQuoteAutosave({
    quoteId: editQuoteId,
    initialDraft,
    getSnapshot,
    getLabel: () =>
      [customerQuery.data?.customer?.name, getValues('quote_number')].filter(Boolean).join(' · '),
    onDraftCreated: id =>
      setSearchParams(
        params => {
          params.set('draft', id)
          return params
        },
        { replace: true }
      ),
    disabled: isTemplate,
  })
  const { touch } = autosave

  useEffect(() => {
    const subscription = watch((_, { type }) => {
      if (type === 'change') touch()
    })
    return () => subscription.unsubscribe()
  }, [watch, touch])

  const editorStateChanges = useRef(0)
  useEffect(() => {
    if (editorStateChanges.current++ > 0) touch()
  }, [
    priceComponentsState,
    selectedAddOns,
    selectedCoupons,
    minimumOverride,
    pendingEntitlements,
    v2,
    touch,
  ])

  const applySnapshot = (snapshot: QuoteFormSeed) => {
    methods.reset(snapshot.values)
    setPricingPlanVersionId(snapshot.values.plan_version_id ?? '')
    setPriceComponentsState(snapshot.priceComponents)
    setSelectedAddOns(snapshot.addOns)
    setSelectedCoupons(snapshot.coupons)
    setMinimumOverride(
      isEdit || hasSlot('quoteCreate.minimumCommitment') ? snapshot.minimumCommitment : undefined
    )
    setPendingEntitlements(snapshot.pendingEntitlements)
    setV2(snapshot.v2)
    setV2Present(v2Presence(snapshot.v2))
  }

  const applyTemplate = (payload: unknown, applied: { id: string; expiresAfterDays?: number }) => {
    const snapshot = fromTemplatePayload(payload)
    if (!snapshot) {
      toast.error('This template could not be read')
      return
    }
    const current = getValues()
    applySnapshot({
      ...snapshot,
      values: {
        ...snapshot.values,
        customer_id: current.customer_id,
        recipients: current.recipients,
        quote_number: current.quote_number,
        start_date: undefined,
        billing_start_date: undefined,
        end_date: undefined,
        valid_until: undefined,
        expires_at:
          applied.expiresAfterDays !== undefined
            ? addDays(startOfDay(new Date()), applied.expiresAfterDays)
            : undefined,
      },
      v2: { ...snapshot.v2, templateId: applied.id, hubspotDealId: v2.hubspotDealId },
    })
  }

  const restoreOffer = () => {
    const snapshot = autosave.acceptOffer()
    if (snapshot) applySnapshot(snapshot)
  }

  const loadTheirVersion = () => {
    const snapshot = autosave.takeTheirs()
    if (snapshot) applySnapshot(snapshot)
  }

  // Auto-disable chargeAutomatically and reset activationCondition when no online provider
  useEffect(() => {
    if (!isLoadingProviders && !hasOnlinePaymentProvider) {
      const currentActivation = getValues('activation_condition')
      const currentCharge = getValues('charge_automatically')

      if (currentActivation === 'ON_CHECKOUT') {
        setValue('activation_condition', 'ON_START')
      }
      if (currentCharge) {
        setValue('charge_automatically', false)
      }
    }
  }, [hasOnlinePaymentProvider, isLoadingProviders, getValues, setValue])

  // Auto-set payment methods type to online when OnCheckout is selected
  useEffect(() => {
    if (activationCondition === 'ON_CHECKOUT' && paymentMethodsType !== 'online') {
      setValue('payment_methods_type', 'online')
    }
  }, [activationCondition, paymentMethodsType, setValue])

  // Auto-disable chargeAutomatically when Bank or External payment methods selected
  useEffect(() => {
    if (
      (paymentMethodsType === 'bankTransfer' || paymentMethodsType === 'external') &&
      chargeAutomatically
    ) {
      setValue('charge_automatically', false)
    }
  }, [paymentMethodsType, chargeAutomatically, setValue])

  const onSubmit = async (data: QuoteFormValues) => {
    try {
      if (!planCurrency) throw new Error('Currency is required')
      if (hasDiscountError) throw new Error('A line discount is above 0 and at most 100')

      const resolvedEntitlements = await resolveEntitlementSpecs(pendingEntitlements, req =>
        createFeatureMutation.mutateAsync(req)
      )

      const createQuoteData = buildCreateQuote({
        values: data,
        currency: planCurrency,
        pricing: priceComponentsState,
        addOns: selectedAddOns,
        coupons: selectedCoupons,
        minimumOverride,
        v2,
        orderedLines,
        entitlements: resolvedEntitlements,
      })

      if (editQuoteId) {
        const force = forceSave.current
        forceSave.current = false
        try {
          await updateQuoteMutation.mutateAsync(
            create(UpdateQuoteRequestSchema, {
              id: editQuoteId,
              quote: createQuoteData,
              expectedUpdatedAt: force ? undefined : baselineUpdatedAt,
            })
          )
        } catch (error) {
          if (ConnectError.from(error).code === Code.Aborted) {
            setStaleSave(true)
            toast.warning(
              'Someone else saved this quote since you opened it: nothing was saved yet.'
            )
            return
          }
          throw error
        }
        void autosave.complete()
        navigate(`${basePath}/quotes/${editQuoteId}`)
      } else {
        const response = await createQuoteMutation.mutateAsync(
          create(CreateQuoteRequestSchema, { quote: createQuoteData })
        )
        void autosave.complete()
        navigate(`${basePath}/quotes/${response.quote?.quote?.id}`)
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unknown error'
      toast.error(`Failed to ${isEdit ? 'update' : 'create'} quote: ${message}`)
    }
  }

  const { previewLines, exampleAmountByKey } = useQuotePreviewLines({
    planComponents: planComponents ?? [],
    pricing: priceComponentsState,
    currency: planCurrency,
    creditUnits,
  })

  const buildPreview = (values: Partial<QuoteFormValues>) =>
    buildPreviewQuote({
      values,
      currency: planCurrency,
      lines: previewLines,
      orderedLines,
      v2,
      addOns: selectedAddOns,
      availableAddOns,
      coupons: selectedCoupons,
      availableCoupons,
      creditUnits,
      defaultNetTerms: netTermsDefault?.netTerms,
      customer: customerQuery.data?.customer,
      invoicingEntity: invoicingEntityQuery.data?.entity,
    })

  const buildTotalsRequest = (values: Partial<QuoteFormValues>) => {
    if (!planCurrency) return undefined
    try {
      return buildCreateQuote({
        values: previewFormValues(values),
        currency: planCurrency,
        pricing: priceComponentsState,
        addOns: selectedAddOns,
        coupons: selectedCoupons,
        minimumOverride,
        v2,
        orderedLines,
        entitlements: [],
      })
    } catch {
      return undefined
    }
  }

  const effectiveMinimum = minimumOverride ?? planDefaultMinimum
  const canPreview =
    (isTemplate || Boolean(customerId)) &&
    Boolean(planVersionId) &&
    pricingValid &&
    !priceComponentsQuery.isLoading
  const canSave = templateEditing
    ? templateEditing.canSave && !templateEditing.isSaving
    : !isSaving && Boolean(customerId) && Boolean(planVersionId) && pricingValid
  const save = templateEditing
    ? () => templateEditing.onSave(toTemplatePayload(getSnapshot()))
    : methods.handleSubmit(onSubmit)
  const saveLabel = templateEditing
    ? templateEditing.isSaving
      ? 'Saving…'
      : 'Save template'
    : submitLabel

  // The preview is a history entry, so the browser's Back returns to the form.
  const previewPushed = useRef(false)
  const previewMode = searchParams.get('preview') === '1' && canPreview
  const openPreview = () => {
    previewPushed.current = true
    setSearchParams(params => {
      params.set('preview', '1')
      return params
    })
  }
  const closePreview = () => {
    if (previewPushed.current) {
      previewPushed.current = false
      navigate(-1)
    } else {
      setSearchParams(
        params => {
          params.delete('preview')
          return params
        },
        { replace: true }
      )
    }
  }

  const totalsBlockedReason =
    !customerId || !planVersionId
      ? 'Needs a customer and a plan'
      : planComponents && !pricingValid
        ? 'Pricing incomplete'
        : hasDiscountError
          ? 'Invalid line discount'
          : undefined
  const totalsLoading = !planCurrency || !planComponents

  if (previewMode) {
    const values = methods.getValues()

    return (
      <div className="flex flex-col gap-6">
        <header className="sticky top-0 z-20 -mx-6 flex flex-wrap items-center justify-between gap-3 border-b border-border bg-background px-6 py-3">
          <h1 className="text-2xl font-semibold">Preview</h1>
          <div className="flex gap-2">
            <Button variant="outline" hasIcon onClick={closePreview}>
              <ArrowLeft className="h-4 w-4" />
              Back to edit
            </Button>
            <Button hasIcon onClick={() => void save()} disabled={!canSave}>
              <Save className="h-4 w-4" />
              {saveLabel}
            </Button>
          </div>
        </header>
        <QuoteFullPreview
          preview={buildPreview(values)}
          totalsRequest={
            totalsBlockedReason || totalsLoading ? undefined : buildTotalsRequest(values)
          }
          minimumCommitment={effectiveMinimum}
        />
      </div>
    )
  }

  const customerName = customerQuery.data?.customer?.name
  const planName = planQuery.data?.plan?.plan?.name
  const title =
    customerName && planName
      ? `${customerName} · ${planName}`
      : customerName || planName || (isEdit && quoteNumber) || 'New quote'

  const editorIssues: SectionIssue[] = [
    ...pricingErrors.map(message => ({ section: 'quote-lines' as const, message })),
    ...(hasDiscountError
      ? [{ section: 'quote-lines' as const, message: 'A line discount is above 0 and at most 100' }]
      : []),
  ]

  const minimumCommitmentField =
    planVersionId &&
    (customerId || isTemplate) &&
    planCurrency &&
    hasSlot('quoteCreate.minimumCommitment') ? (
      <Slot
        name="quoteCreate.minimumCommitment"
        currency={planCurrency}
        allowPending
        components={scopeOptionsWithPending([
          ...(planComponents ?? []).filter(
            c => !priceComponentsState.components.removed.includes(c.id)
          ),
          ...priceComponentsState.components.extra.map(e => ({
            name: e.name,
            productId: e.productId,
          })),
        ])}
        planDefault={planDefaultMinimum}
        override={minimumOverride}
        dropped={false}
        allowDrop={false}
        onChange={({ override }) => setMinimumOverride(override)}
      />
    ) : undefined

  return (
    <Form {...methods}>
      {/* Saving is the header's explicit action only: an untyped button or Enter never submits. */}
      <form onSubmit={e => e.preventDefault()} className="flex flex-col gap-5">
        {templateEditing ? (
          <QuoteEditorHeader
            crumbs={templateEditing.crumbs}
            title={templateEditing.title}
            details={templateEditing.details}
            onPreview={openPreview}
            previewDisabled={!canPreview}
            onSave={() => void save()}
            saveDisabled={!canSave}
            saveLabel={saveLabel}
          />
        ) : (
          <QuoteEditorHeader
            crumbs={[
              { label: 'Quotes', to: `${basePath}/quotes` },
              editQuoteId
                ? { label: quoteNumber || 'Quote', to: `${basePath}/quotes/${editQuoteId}` }
                : { label: 'New quote' },
            ]}
            title={title}
            subtitle={template ? `Based on quote ${template.sourceQuoteNumber}` : undefined}
            status={status}
            autosaveStatus={autosave.status}
            templates={
              <Slot
                name="quoteCreate.templates"
                snapshot={() => toTemplatePayload(getSnapshot())}
                apply={applyTemplate}
                isEdit={isEdit}
              />
            }
            onPreview={openPreview}
            previewDisabled={!canPreview}
            onSave={() => void save()}
            saveDisabled={!canSave}
            saveLabel={saveLabel}
          />
        )}

        {staleSave && editQuoteId && (
          <StaleQuoteBanner
            savedVersionHref={`${basePath}/quotes/${editQuoteId}`}
            onOverwrite={() => {
              forceSave.current = true
              setStaleSave(false)
              void save()
            }}
          />
        )}
        {autosave.conflict && (
          <DraftConflictBanner
            draft={autosave.conflict}
            onLoadTheirs={loadTheirVersion}
            onKeepMine={autosave.keepMine}
          />
        )}
        {autosave.offer && (
          <DraftRestoreBanner
            offer={autosave.offer}
            onRestore={restoreOffer}
            onDiscard={() => void autosave.discardOffer()}
          />
        )}
        {template?.planOutdated && (
          <Alert variant="warning">
            The plan of quote {template.sourceQuoteNumber} has a newer version, so its pricing could
            not be reused. Select a plan to configure pricing.
          </Alert>
        )}

        <div className="@container pb-4">
          <div className="grid grid-cols-1 items-start gap-6 @4xl:grid-cols-[minmax(0,1fr)_320px] @6xl:grid-cols-[200px_minmax(0,1fr)_340px]">
            <aside className="sticky top-28 hidden @6xl:block">
              <QuoteEditorOutline editorIssues={editorIssues} template={isTemplate} />
            </aside>

            <div className="flex min-w-0 flex-col gap-5">
              <PeopleSection
                v2={v2}
                setV2={setV2}
                allowMultipleRecipients={multipleSigners}
                allowVerified={v2Gates.verified}
                allowCountersigners={v2Gates.countersigners}
                allowCustomerActions={v2Gates.customerActions}
                onRecipientsChange={touch}
                dealFields={!isTemplate}
              />

              <LinesSection
                customerId={customerId}
                planVersionId={planVersionId}
                currency={planCurrency}
                planComponents={planComponents}
                pricing={priceComponentsState}
                setPricing={setPriceComponentsState}
                pricingErrors={pricingErrors}
                lines={orderedLines}
                addOns={selectedAddOns}
                setAddOns={setSelectedAddOns}
                availableAddOns={availableAddOns}
                coupons={selectedCoupons}
                setCoupons={setSelectedCoupons}
                availableCoupons={availableCoupons}
                exampleAmountByKey={exampleAmountByKey}
                v2={v2}
                setV2={setV2}
                lineEditing={v2Gates.lineEditing}
                minimumCommitment={minimumCommitmentField}
                requireCustomer={!isTemplate}
              />

              {env.entitlementsEnabled && planVersionId && !isEdit && (
                <EntitlementsBlock
                  selection={{
                    planVersionId,
                    addOnIds: selectedAddOns.map(a => a.addOnId),
                    extraProductIds,
                    removedProductIds,
                  }}
                  pending={pendingEntitlements}
                  onChange={setPendingEntitlements}
                />
              )}

              <DocumentSection sections={v2.sections} attachments={v2.attachments} setV2={setV2} />

              <TermsSection
                planVersionId={planVersionId}
                trialConfig={planTrialConfig}
                trialingPlanName={trialingPlanName}
                isFreePlan={isFreePlan}
                netTermsDefault={netTermsDefault}
                hasOnlinePaymentProvider={hasOnlinePaymentProvider}
                isLoadingProviders={isLoadingProviders}
                hideDates={isTemplate}
              />

              {!isTemplate && (
                <InternalSection
                  hubspotDealId={v2.hubspotDealId}
                  setV2={setV2}
                  allowHubspot={v2Gates.hubspot}
                />
              )}
            </div>

            <aside aria-label="Live preview" className="min-w-0 @4xl:sticky @4xl:top-28">
              <QuoteEditorRail
                buildPreview={buildPreview}
                buildTotalsRequest={buildTotalsRequest}
                totalsBlockedReason={totalsBlockedReason}
                totalsLoading={totalsLoading}
                minimumCommitment={effectiveMinimum}
                canOpenPreview={canPreview}
                onOpenPreview={openPreview}
                showSummary={!isTemplate}
              />
            </aside>
          </div>
        </div>
      </form>
    </Form>
  )
}
