import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  Input,
  Label,
  ScrollArea,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ArrowLeftIcon, PackageIcon, PlusIcon } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'

import { ChargeComposer } from '@/features/plans/charges/ChargeComposer'
import {
  PRESETS,
  chargeDefaults,
  compileCharge,
  decompileCharge,
} from '@/features/plans/charges/compile'
import { useChargeForm } from '@/features/plans/charges/useChargeForm'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { useCurrency } from '@/features/plans/pricecomponents/utils'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  wrapAsNewPriceEntries,
} from '@/features/pricing/conversions'
import {
  buildDefaults,
  extractStructuralInfo,
  protoFeeTypeToComponentFeeType,
} from '@/features/pricing/formData'
import { rpcErrorMessage } from '@/lib/rpcError'
import { getResolvedEntitlementsForPlanVersion } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  createPriceComponent,
  listPriceComponents,
} from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

import { ProductBrowser } from './ProductBrowser'

import type { ChargeLock, ChargeShape, ChargeValues } from '@/features/plans/charges/compile'
import type { ProductWithPrice } from '@/rpc/api/products/v1/models_pb'

type PickedProduct = { kind: 'new'; name: string } | { kind: 'existing'; item: ProductWithPrice }

export const AddComponentPanel = () => {
  const navigate = useNavigate()
  // The server takes new charges in the version currency; variants price the others.
  const currency = useCurrency()
  const [picked, setPicked] = useState<PickedProduct | null>(null)

  return (
    <Sheet open={true} onOpenChange={() => navigate('..')}>
      <SheetContent size="large">
        <SheetHeader className="border-b border-border pb-3 mb-3">
          <SheetTitle>Add a charge</SheetTitle>
          <SheetDescription>
            {picked
              ? 'How many, how much and when: pick a preset or answer the three questions.'
              : 'Choose the product this charge sells, or create one.'}
          </SheetDescription>
        </SheetHeader>
        <ScrollArea className="h-[calc(100%-80px)]">
          <div className="pr-3 pb-6">
            {picked ? (
              <ChargeStep
                key={picked.kind === 'new' ? `new:${picked.name}` : picked.item.product?.id}
                picked={picked}
                currency={currency}
                onBack={() => setPicked(null)}
              />
            ) : (
              <ProductStep currency={currency} onPick={setPicked} />
            )}
          </div>
        </ScrollArea>
      </SheetContent>
    </Sheet>
  )
}

// --- Step 1: the product ---

const ProductStep = ({
  currency,
  onPick,
}: {
  currency: string
  onPick: (picked: PickedProduct) => void
}) => {
  const [name, setName] = useState('')
  const createNew = () => name.trim() && onPick({ kind: 'new', name: name.trim() })

  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <Label htmlFor="new-product-name">New product</Label>
        <div className="flex gap-2">
          <Input
            id="new-product-name"
            placeholder="e.g. Platform, Seats, API calls"
            value={name}
            onChange={e => setName(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && createNew()}
          />
          <Button type="button" variant="brand" hasIcon disabled={!name.trim()} onClick={createNew}>
            <PlusIcon size={14} /> Create
          </Button>
        </div>
      </div>
      <div className="space-y-2">
        <span className="text-xs font-medium text-muted-foreground">Or reuse a product</span>
        <ProductBrowser
          currency={currency}
          onSelect={item => onPick({ kind: 'existing', item })}
        />
      </div>
    </div>
  )
}

// --- Step 2: the charge ---

const DEFAULT_SHAPE = PRESETS[0].shape as ChargeShape

const ChargeStep = ({
  picked,
  currency,
  onBack,
}: {
  picked: PickedProduct
  currency: string
  onBack: () => void
}) => {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { version } = usePlanWithVersion()

  const product = picked.kind === 'existing' ? picked.item.product : undefined
  const productName = product?.name ?? (picked.kind === 'new' ? picked.name : '')

  const { lock, structural, initial } = useMemo(() => {
    if (!product || product.feeType === undefined) {
      return {
        lock: undefined,
        structural: undefined,
        initial: { shape: DEFAULT_SHAPE, values: chargeDefaults(DEFAULT_SHAPE) },
      }
    }
    const feeType = protoFeeTypeToComponentFeeType(product.feeType)
    const structural = extractStructuralInfo(feeType, product.feeStructure)
    const lock: ChargeLock = {
      feeType,
      billingType: structural.billingType as ChargeLock['billingType'],
    }
    const latestPrice = picked.kind === 'existing' ? (picked.item.latestPrice ?? undefined) : undefined
    const formData = buildDefaults(feeType, latestPrice, false, structural.usageModel)
    const initial = decompileCharge(feeType, {
      ...formData,
      metricId: structural.metricId,
      slotUnitName: structural.slotUnitName,
      billingType: structural.billingType,
    })
    return { lock, structural, initial }
    // The step is keyed by product: it is computed once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const [shape, setShape] = useState<ChargeShape>(initial.shape)
  const [include, setInclude] = useState(false)
  const methods = useChargeForm({
    shape,
    defaultValues: initial.values,
    currency,
    newProduct: !product,
  })

  const createComponent = useMutation(createPriceComponent, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listPriceComponents, cardinality: undefined }),
      })
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getResolvedEntitlementsForPlanVersion.parent,
          cardinality: undefined,
        }),
      })
      navigate('..')
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The charge could not be added')),
  })

  const submit = methods.handleSubmit(values => {
    if (!version?.id) return
    const compiled = compileCharge(shape, values as ChargeValues, lock)
    const priceInputs = buildPriceInputs(compiled.pricingType, compiled.formData, currency)
    createComponent.mutate({
      planVersionId: version.id,
      name: productName,
      product: product
        ? buildExistingProductRef(product.id)
        : buildNewProductRef(productName, compiled.feeType, compiled.formData),
      prices: wrapAsNewPriceEntries(priceInputs),
    })
  })

  const addIncluded = () => {
    if (!version?.id || !product) return
    createComponent.mutate({
      planVersionId: version.id,
      name: product.name,
      product: buildExistingProductRef(product.id),
      prices: [],
    })
  }

  return (
    <div className="space-y-5">
      <div className="flex items-center gap-3 rounded-lg border border-border bg-card px-3 py-2">
        <PackageIcon size={16} className="text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">{productName}</div>
          <div className="text-xs text-muted-foreground">
            {product ? 'Existing product' : 'New product'}
          </div>
        </div>
        <Button type="button" variant="ghost" size="sm" hasIcon onClick={onBack}>
          <ArrowLeftIcon size={14} /> Change
        </Button>
      </div>

      <Form {...methods}>
        <ChargeComposer
          shape={shape}
          onShapeChange={setShape}
          methods={methods}
          currency={currency}
          lock={lock}
          structural={structural}
          include={include}
          onIncludeChange={product ? setInclude : undefined}
        />
      </Form>

      <div className="flex justify-end gap-2 pt-2">
        <Button type="button" variant="secondary" onClick={() => navigate('..')}>
          Cancel
        </Button>
        <Button
          type="button"
          variant="primary"
          disabled={createComponent.isPending}
          onClick={include ? addIncluded : submit}
        >
          {include ? 'Add without price' : 'Add charge'}
        </Button>
      </div>
    </div>
  )
}
