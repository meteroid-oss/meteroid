import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  ComboboxFormField,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Form,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ArchiveIcon, PencilIcon, PlusIcon } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import ConfirmationModal from '@/components/ConfirmationModal'
import {
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import {
  archiveCreditUnitPrice,
  setCreditUnitProduct,
} from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'
import {
  getProduct,
  searchProducts,
} from '@/rpc/api/products/v1/products-ProductsService_connectquery'

import { CreditUnitPriceDialog } from './CreditUnitPriceDialog'
import { CREDIT_VALUE_HELP } from './creditValue'
import { invalidateCreditUnits } from './queries'

import type { CreditUnit, CreditUnitPrice } from '@/rpc/api/creditunits/v1/models_pb'

const productSchema = z.object({ productId: z.string() })

/** Where a credit unit is sold: its price per currency and the product its sales are taxed as. */
export const CreditUnitSellingDialog = ({
  unit,
  onClose,
}: {
  unit: CreditUnit
  onClose: () => void
}) => {
  const queryClient = useQueryClient()
  const [priceDialog, setPriceDialog] = useState<{ currency?: string } | null>(null)
  const [priceToArchive, setPriceToArchive] = useState<CreditUnitPrice | null>(null)

  const archivePrice = useMutation(archiveCreditUnitPrice, {
    onSuccess: async () => {
      await invalidateCreditUnits(queryClient)
      toast.success('Selling price removed')
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The selling price could not be removed')),
    onSettled: () => setPriceToArchive(null),
  })

  const prices = [...unit.prices].sort((a, b) => a.currency.localeCompare(b.currency))

  return (
    <>
      <Dialog open onOpenChange={open => !open && onClose()}>
        <DialogContent className="sm:max-w-[560px] max-h-[90vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>Sell {unit.pluralName}</DialogTitle>
            <DialogDescription>
              Customers buy {unit.pluralName} at the price in their currency, plus tax.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-6">
            <SettingsSection
              divided={false}
              title="Value per credit"
              description={`One value per currency. ${CREDIT_VALUE_HELP} Customers in other currencies cannot buy ${unit.pluralName}.`}
              action={
                <Button
                  size="sm"
                  variant="outline"
                  className="gap-2"
                  onClick={() => setPriceDialog({})}
                >
                  <PlusIcon className="h-4 w-4" />
                  Add price
                </Button>
              }
            >
              {prices.length === 0 ? (
                <div className="rounded-lg border border-border p-6 text-center text-xs text-muted-foreground">
                  No money value yet: overage in {unit.pluralName} is not billed unless a plan sets
                  its own price, and they cannot be sold.
                </div>
              ) : (
                <SettingsRowGroup>
                  {prices.map(price => (
                    <SettingsRow
                      key={price.id}
                      title={<span className="font-mono">{price.currency}</span>}
                      description={`${formatCurrencyNoRounding(price.unitPrice, price.currency)} per ${unit.name}`}
                    >
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`Change the ${price.currency} price`}
                        onClick={() => setPriceDialog({ currency: price.currency })}
                      >
                        <PencilIcon className="h-4 w-4" />
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`Remove the ${price.currency} price`}
                        className="text-destructive hover:text-destructive"
                        onClick={() => setPriceToArchive(price)}
                      >
                        <ArchiveIcon className="h-4 w-4" />
                      </Button>
                    </SettingsRow>
                  ))}
                </SettingsRowGroup>
              )}
            </SettingsSection>

            <TaxProductSection unit={unit} />
          </div>
        </DialogContent>
      </Dialog>

      {priceDialog && (
        <CreditUnitPriceDialog
          unit={unit}
          initialCurrency={priceDialog.currency}
          onClose={() => setPriceDialog(null)}
        />
      )}

      <ConfirmationModal
        visible={!!priceToArchive}
        onSelectConfirm={() =>
          priceToArchive &&
          archivePrice.mutate({ creditUnitId: unit.id, priceId: priceToArchive.id })
        }
        onSelectCancel={() => setPriceToArchive(null)}
        header={`Remove the ${priceToArchive?.currency ?? ''} price`}
        danger
        buttonLabel="Remove price"
      >
        Customers billed in {priceToArchive?.currency} can no longer buy {unit.pluralName}: top-ups
        and automatic top-ups in that currency stop until a new price is set. Credits already bought
        are not affected.
      </ConfirmationModal>
    </>
  )
}

const TaxProductSection = ({ unit }: { unit: CreditUnit }) => {
  const queryClient = useQueryClient()
  const [search, setSearch] = useState('')
  const debouncedSearch = useDebounceValue(search, 300)

  const methods = useZodForm({
    schema: productSchema,
    defaultValues: { productId: unit.productId ?? '' },
  })
  const productId = useWatch({ control: methods.control, name: 'productId' })

  const { reset } = methods
  useEffect(() => {
    reset({ productId: unit.productId ?? '' })
  }, [unit.productId, reset])

  const productsQuery = useQuery(searchProducts, {
    query: debouncedSearch.trim() || undefined,
    archived: false,
    catalogOnly: true,
    pagination: { page: 0, perPage: 50 },
  })
  // The saved product may fall outside the current search page; it must still show its name.
  const currentProduct = useQuery(
    getProduct,
    { productId: unit.productId ?? '' },
    { enabled: !!unit.productId }
  ).data?.product

  const options = useMemo(() => {
    const found = (productsQuery.data?.products ?? []).map(product => ({
      value: product.id,
      label: product.name,
      keywords: [product.name],
    }))
    if (currentProduct && !found.some(option => option.value === currentProduct.id)) {
      found.unshift({
        value: currentProduct.id,
        label: currentProduct.name,
        keywords: [currentProduct.name],
      })
    }
    return found
  }, [productsQuery.data, currentProduct])

  const setProduct = useMutation(setCreditUnitProduct, {
    onSuccess: async () => {
      await invalidateCreditUnits(queryClient)
      toast.success('Tax product saved')
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The tax product could not be saved')),
  })

  const onSubmit = (values: z.infer<typeof productSchema>) =>
    setProduct.mutateAsync({
      creditUnitId: unit.id,
      productId: values.productId || undefined,
    })

  const unchanged = productId === (unit.productId ?? '')

  return (
    <SettingsSection
      title="Tax product"
      description={
        !unit.productId
          ? `Required to sell ${unit.pluralName}. Sets the tax category and accounting code.`
          : currentProduct?.archivedAt
            ? `This product is archived, so ${unit.pluralName} cannot be sold: choose an active product.`
            : 'Sets the tax category and accounting code of credit sales.'
      }
    >
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="flex items-end gap-3">
          <ComboboxFormField
            control={methods.control}
            name="productId"
            label="Product"
            placeholder="Choose a product"
            hasSearch
            clearable
            shouldFilter={false}
            onSearchChange={setSearch}
            unit="product"
            options={options}
            containerClassName="flex-1"
          />
          <Button type="submit" disabled={unchanged || setProduct.isPending}>
            {setProduct.isPending ? 'Saving...' : 'Save'}
          </Button>
        </form>
      </Form>
      {!productId && unit.productId && (
        <p className="mt-2 text-xs text-muted-foreground">
          Saving without a product stops all sales of {unit.pluralName}.
        </p>
      )}
    </SettingsSection>
  )
}
