import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { MAX_UNIT_PRICE_DECIMALS, unitPriceSchema } from '@/features/credit/unitSale'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { rpcErrorMessage } from '@/lib/rpcError'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { setCreditUnitPrice } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import { CREDIT_VALUE_HELP } from './creditValue'
import { invalidateCreditUnits } from './queries'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'

const schema = z.object({
  currency: z.string().min(1, 'Choose a currency'),
  unitPrice: unitPriceSchema,
})

type Values = z.infer<typeof schema>

export const CreditUnitPriceDialog = ({
  unit,
  initialCurrency,
  onClose,
}: {
  unit: CreditUnit
  initialCurrency?: string
  onClose: () => void
}) => {
  const queryClient = useQueryClient()
  const currencies = useQuery(listTenantCurrencies).data?.currencies ?? []
  const initialPrice = unit.prices.find(price => price.currency === initialCurrency)

  const methods = useZodForm({
    schema,
    defaultValues: {
      currency: initialCurrency ?? '',
      unitPrice: initialPrice?.unitPrice ?? '',
    },
  })
  const currency = useWatch({ control: methods.control, name: 'currency' })
  const replaced = unit.prices.find(price => price.currency === currency)

  const setPrice = useMutation(setCreditUnitPrice, {
    onSuccess: async () => {
      await invalidateCreditUnits(queryClient)
      toast.success('Selling price saved')
      onClose()
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The selling price could not be saved')),
  })

  const onSubmit = (values: Values) =>
    setPrice.mutateAsync({
      creditUnitId: unit.id,
      currency: values.currency,
      unitPrice: values.unitPrice.trim(),
    })

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>
            {initialPrice ? 'Change value per credit' : 'Add value per credit'}
          </DialogTitle>
          <DialogDescription>
            What one {unit.name} is worth in this currency, before tax. {CREDIT_VALUE_HELP}
          </DialogDescription>
        </DialogHeader>
        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
            {initialCurrency ? (
              <div className="space-y-1 text-sm">
                <div className="font-medium">Currency</div>
                <div className="text-muted-foreground">{initialCurrency}</div>
              </div>
            ) : (
              <SelectFormField
                control={methods.control}
                name="currency"
                label="Currency"
                placeholder="Choose a currency"
              >
                {currencies.map(code => (
                  <SelectItem key={code} value={code}>
                    {code}
                  </SelectItem>
                ))}
              </SelectFormField>
            )}
            <InputFormField
              control={methods.control}
              name="unitPrice"
              label={currency ? `Price per ${unit.name} (${currency})` : `Price per ${unit.name}`}
              placeholder="e.g. 0.001"
              inputMode="decimal"
              autoFocus
              description={
                replaced
                  ? `Replaces ${formatCurrencyNoRounding(replaced.unitPrice, replaced.currency)} for new sales.`
                  : `Up to ${MAX_UNIT_PRICE_DECIMALS} decimals.`
              }
            />
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" disabled={setPrice.isPending}>
                {setPrice.isPending ? 'Saving...' : 'Save price'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
