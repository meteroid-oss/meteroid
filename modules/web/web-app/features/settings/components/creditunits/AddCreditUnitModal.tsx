import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  CheckboxFormField,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { unitPriceSchema } from '@/features/credit/unitSale'
import { useTenant } from '@/hooks/useTenant'
import { useZodForm } from '@/hooks/useZodForm'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  createCreditUnit,
  setCreditUnitPrice,
} from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'

import { CREDIT_VALUE_HELP } from './creditValue'
import { invalidateCreditUnits } from './queries'

// Mirrors the store's `CreditUnitCode` rule and the DB CHECK: 4-16 chars, A-Z/0-9/_, starting
// with a letter. A 3-letter code is refused server-side too (it would clash with an ISO code).
const codeSchema = z
  .string()
  .trim()
  .toUpperCase()
  .regex(/^[A-Z][A-Z0-9_]{3,15}$/, 'Code must be 4-16 characters: letters, digits or _, starting with a letter')

const creditUnitSchema = z
  .object({
    code: codeSchema,
    name: z.string().min(1, 'Name is required').max(64),
    pluralName: z.string().min(1, 'Plural name is required').max(64),
    symbol: z.string().max(8, 'At most 8 characters').optional(),
    precision: z.coerce.number().int().min(0).max(6),
    valuePerCredit: z.string(),
    noMoneyValue: z.boolean(),
  })
  .superRefine((values, ctx) => {
    if (values.noMoneyValue) return
    const value = unitPriceSchema.safeParse(values.valuePerCredit)
    if (!value.success) {
      ctx.addIssue({
        code: 'custom',
        path: ['valuePerCredit'],
        message: `${value.error.issues[0]?.message ?? 'Enter a value per credit'}, or tick "No money value"`,
      })
    }
  })

type CreditUnitFormData = z.infer<typeof creditUnitSchema>

interface AddCreditUnitModalProps {
  open: boolean
  onClose: () => void
  /** Opens where the value of the unit `unitId` is set. */
  onSetValue: (unitId: string) => void
}

export const AddCreditUnitModal = ({ open, onClose, onSetValue }: AddCreditUnitModalProps) => {
  const queryClient = useQueryClient()
  const currency = useTenant().tenant?.reportingCurrency

  const createCreditUnitMut = useMutation(createCreditUnit)
  const setPriceMut = useMutation(setCreditUnitPrice)

  const methods = useZodForm({
    schema: creditUnitSchema,
    defaultValues: {
      code: '',
      name: '',
      pluralName: '',
      symbol: '',
      precision: 0,
      valuePerCredit: '',
      noMoneyValue: false,
    },
  })
  const noMoneyValue = useWatch({ control: methods.control, name: 'noMoneyValue' })
  const singular = useWatch({ control: methods.control, name: 'name' })

  const done = async () => {
    await invalidateCreditUnits(queryClient)
    onClose()
    methods.reset()
  }

  const onSubmit = async (data: CreditUnitFormData) => {
    if (!data.noMoneyValue && !currency) return
    let unitId: string | undefined
    try {
      unitId = (
        await createCreditUnitMut.mutateAsync({
          code: data.code,
          name: data.name,
          pluralName: data.pluralName,
          symbol: data.symbol?.trim() ? data.symbol.trim() : undefined,
          precision: data.precision,
        })
      ).unit?.id
    } catch (error) {
      toast.error(rpcErrorMessage(error, 'The credit unit could not be created'))
      return
    }
    if (!data.noMoneyValue && unitId && currency) {
      const created = unitId
      try {
        await setPriceMut.mutateAsync({
          creditUnitId: created,
          currency,
          unitPrice: data.valuePerCredit.trim(),
        })
      } catch (error) {
        toast.error(
          `The ${data.code} credit unit was created without a value (${rpcErrorMessage(error, 'its value was not saved')}).`,
          { action: { label: 'Set its value', onClick: () => onSetValue(created) } }
        )
        await done()
        return
      }
    }
    toast.success('Credit unit created')
    await done()
  }
  const pending = createCreditUnitMut.isPending || setPriceMut.isPending
  const currencyUnknown = !noMoneyValue && !currency

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>Add credit unit</DialogTitle>
          <DialogDescription>A denomination to price usage in, instead of money.</DialogDescription>
        </DialogHeader>

        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
            <InputFormField
              name="code"
              label="Code"
              placeholder="CREDITS"
              description="Immutable. 4-16 characters: letters, digits or underscore."
              control={methods.control}
              className="uppercase"
            />

            <div className="grid grid-cols-2 gap-4">
              <InputFormField
                name="name"
                label="Name (singular)"
                placeholder="Credit"
                control={methods.control}
              />
              <InputFormField
                name="pluralName"
                label="Name (plural)"
                placeholder="Credits"
                control={methods.control}
              />
            </div>

            <div className="grid grid-cols-2 gap-4">
              <InputFormField
                name="symbol"
                label="Symbol (optional)"
                placeholder="cr"
                control={methods.control}
              />
              <InputFormField
                name="precision"
                label="Precision"
                type="number"
                description="Decimal places. Immutable. 0 to 6."
                control={methods.control}
              />
            </div>

            <InputFormField
              name="valuePerCredit"
              label={`Value per ${singular.trim() || 'credit'}${currency ? ` (${currency})` : ''}`}
              placeholder="e.g. 0.01"
              inputMode="decimal"
              disabled={noMoneyValue}
              description={CREDIT_VALUE_HELP}
              control={methods.control}
            />
            <CheckboxFormField
              name="noMoneyValue"
              label="No money value (free allowance only)"
              description="Overage is then not billed unless a plan sets its own price per credit, and the credits cannot be sold."
              control={methods.control}
            />

            <DialogFooter>
              <Button variant="outline" type="button" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" disabled={pending || currencyUnknown}>
                {pending ? 'Creating...' : 'Create credit unit'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
