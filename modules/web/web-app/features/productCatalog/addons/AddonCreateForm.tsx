import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Textarea,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useId, useState, type ReactNode } from 'react'
import { toast } from 'sonner'

import { AdvancedSection } from '@/components/form'
import { NewChargeForm } from '@/features/plans/charges/ChargeForm'
import { TERMS } from '@/features/plans/currencies/priceCells'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
} from '@/features/pricing/conversions'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { createAddOn, listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import { PerSubscriptionField, SelfServiceField } from './AddonSettingsFields'

import type { NewCharge } from '@/features/plans/charges/ChargeForm'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'

interface Props {
  /** The currency and billing period its first price starts in: the plan's, from a plan. */
  defaultCurrency?: string
  defaultCadence?: BillingPeriod
  /** The product line a new product goes to and its metric comes from: the plan's, from a plan. */
  familyLocalId?: string
  submitLabel: string
  onCancel: () => void
  onCreated: (addOn: AddOn) => void | Promise<unknown>
}

/**
 * "New add-on": its first price (where it is sold and how often), how it is sold, then its name
 * and charge, as a plan prices one. Other currencies and billing periods are priced once created.
 */
export const AddonCreateForm = ({
  defaultCurrency,
  defaultCadence = BillingPeriod.MONTHLY,
  familyLocalId,
  submitLabel,
  onCancel,
  onCreated,
}: Props) => {
  const queryClient = useQueryClient()
  const descriptionId = useId()

  const currenciesQuery = useQuery(listTenantCurrencies)
  const currencies = currenciesQuery.data?.currencies ?? []
  const familiesQuery = useQuery(listProductFamilies)
  const families = [...(familiesQuery.data?.productFamilies ?? [])].sort((a, b) =>
    a.id > b.id ? 1 : -1
  )
  const [pickedCurrency, setCurrency] = useState<string>()
  const [cadence, setCadence] = useState(defaultCadence)
  const [pickedFamily, setFamily] = useState<string>()
  const [maxInstances, setMaxInstances] = useState<number | undefined>(1)
  const [selfServiceable, setSelfServiceable] = useState(false)
  const [description, setDescription] = useState('')

  const currency = pickedCurrency ?? defaultCurrency ?? currencies[0]
  const family = pickedFamily ?? familyLocalId ?? families[0]?.localId

  const create = useMutation(createAddOn)

  const submit = async ({ name, product, charge }: NewCharge) => {
    if (charge.included || !currency) return
    // Its first price; capacity levels beyond the first are not sold on an add-on.
    const [first] = buildPriceInputs(charge.pricingType, charge.formData, currency)
    if (!first) return
    try {
      const created = await create.mutateAsync({
        name,
        description: description.trim() || undefined,
        product: product
          ? buildExistingProductRef(product.id)
          : buildNewProductRef(name, charge.feeType, charge.formData),
        prices: [first],
        selfServiceable,
        maxInstancesPerSubscription: maxInstances,
        productFamilyLocalId: family,
        entitlements: [],
      })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listAddOns.parent, cardinality: undefined }),
      })
      if (created.addOn) await onCreated(created.addOn)
    } catch (error) {
      toast.error(rpcErrorMessage(error, 'The add-on could not be created'))
    }
  }

  // The semiannual cadence only where it is the one asked for.
  const terms = TERMS.filter(t => t.term !== BillingPeriod.SEMIANNUAL || t.term === defaultCadence)
  const familyName = families.find(f => f.localId === family)?.name

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-end gap-3">
        <Field label="Priced in">
          <Select value={currency} onValueChange={setCurrency}>
            <SelectTrigger className="h-8 w-[110px]" aria-label="Currency">
              <SelectValue placeholder="Currency" />
            </SelectTrigger>
            <SelectContent>
              {currencies.map(c => (
                <SelectItem key={c} value={c}>
                  {c}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <Field label="Billed">
          <Select value={String(cadence)} onValueChange={value => setCadence(Number(value) as BillingPeriod)}>
            <SelectTrigger className="h-8 w-[130px]" aria-label="Billed">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {terms.map(t => (
                <SelectItem key={t.term} value={String(t.term)}>
                  {t.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Field>
        <p className="min-w-0 flex-1 basis-40 pb-1.5 text-xs text-muted-foreground">
          Its first price. Other currencies and billing periods are priced once it is created.
        </p>
      </div>

      <div className="space-y-2.5">
        <PerSubscriptionField value={maxInstances} onChange={setMaxInstances} />
        <SelfServiceField value={selfServiceable} onChange={setSelfServiceable} />
      </div>

      <AdvancedSection
        label="More settings"
        summary={[description.trim() && 'Described', families.length > 1 && familyName]
          .filter(Boolean)
          .join(' · ')}
      >
        <div className="space-y-1.5">
          <Label htmlFor={descriptionId} className="text-xs font-normal text-muted-foreground">
            Description
          </Label>
          <Textarea
            id={descriptionId}
            rows={2}
            placeholder="Optional, shown where customers pick it"
            value={description}
            onChange={e => setDescription(e.target.value)}
          />
        </div>
        {families.length > 1 && (
          <Field label="Product line of a new product">
            <Select value={family} onValueChange={setFamily}>
              <SelectTrigger className="h-8 w-[220px]" aria-label="Product line">
                <SelectValue placeholder="Product line" />
              </SelectTrigger>
              <SelectContent>
                {families.map(f => (
                  <SelectItem key={f.localId} value={f.localId}>
                    {f.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        )}
      </AdvancedSection>

      <div className="space-y-4 border-t border-border pt-4">
        {currency ? (
          <NewChargeForm
            // Its kind and fields start on the billing period: picked again, they start over.
            key={cadence}
            currency={currency}
            cadence={cadence}
            familyLocalId={family}
            anyFamily
            creditPack
            // An add-on's seats would share the plan's count and ignore its quantity.
            hiddenKinds={['seat']}
            context={`First price · ${currency}`}
            submitLabel={submitLabel}
            pending={create.isPending}
            onCancel={onCancel}
            onSubmit={submit}
          />
        ) : (
          !currenciesQuery.isLoading && (
            <p className="text-sm text-muted-foreground">Add a currency to your account first.</p>
          )
        )}
      </div>
    </div>
  )
}

const Field = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="space-y-1">
    <span className="block text-xs text-muted-foreground">{label}</span>
    {children}
  </div>
)
