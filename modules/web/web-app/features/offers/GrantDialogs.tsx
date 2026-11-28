import { create } from '@bufbuild/protobuf'
import {
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
  MultiSelectFormField,
  MultiSelectItem,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { useState } from 'react'
import { z } from 'zod'

import { CurrencySelect } from '@/components/CurrencySelect'
import { ChoiceCardsFormField } from '@/components/form'
import { FeatureCreateSheet } from '@/features/entitlements/features/FeatureCreateSheet'
import { FeaturePicker, ValueField } from '@/features/entitlements/rows/RowSheet'
import { valueFromForm, valueIssue, valueToForm } from '@/features/entitlements/rows/rowForm'
import { useFeatures } from '@/features/entitlements/rows/useFeatures'
import {
  CouponDiscountField,
  OtherAmountsField,
} from '@/features/productCatalog/coupons/CouponTermsFields'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import {
  EntitlementEffect,
  EntitlementValueSchema,
  FeatureKind,
  FeatureValueType,
} from '@/rpc/api/entitlements/v1/models_pb'

import { GRANT_KIND_LABELS } from './grants'

import type { GrantDefinition, GrantKind } from './grants'
import type { ChoiceCardOption } from '@/components/form'
import type { RowForm } from '@/features/entitlements/rows/rowForm'
import type { CouponTermsFormFields } from '@/features/productCatalog/coupons/CouponTermsFields'
import type { Feature } from '@/rpc/api/entitlements/v1/models_pb'
import type { ReactNode } from 'react'
import type { Control } from 'react-hook-form'

interface DialogProps<K extends GrantKind> {
  initial?: GrantDefinition<K>
  onSave: (definition: GrantDefinition<K>) => void
  onClose: () => void
}

const isPositive = (value: string | undefined) => !!value?.trim() && Number(value) > 0

const GrantDialogShell = ({
  kind,
  editing,
  onClose,
  onSubmit,
  children,
}: {
  kind: GrantKind
  editing: boolean
  onClose: () => void
  onSubmit: (event: React.FormEvent<HTMLFormElement>) => void
  children: ReactNode
}) => (
  <Dialog open onOpenChange={open => !open && onClose()}>
    <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-[520px]">
      <DialogHeader>
        <DialogTitle>
          {`${editing ? 'Edit' : 'Add'} ${GRANT_KIND_LABELS[kind].toLowerCase()}`}
        </DialogTitle>
      </DialogHeader>
      <form
        onSubmit={event => {
          // Opened inside the offer's or the plan's form: its submit must not reach that form.
          event.stopPropagation()
          onSubmit(event)
        }}
        className="space-y-4"
      >
        {children}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" variant="primary">
            {editing ? 'Save' : 'Add'}
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  </Dialog>
)

// ── Credit ────────────────────────────────────────────────────────────────────

const creditSchema = z.object({
  amount: z.string().trim().refine(isPositive, { message: 'Enter a positive amount' }),
  currency: z.string().min(1, 'Select a currency'),
  scope: z.enum(['customer', 'subscription']),
})

const SCOPE_OPTIONS: ChoiceCardOption<'customer' | 'subscription'>[] = [
  { value: 'customer', label: 'The customer', hint: 'Pays any of its subscriptions' },
  { value: 'subscription', label: 'This subscription', hint: 'Ends with it' },
]

export const CreditGrantDialog = ({
  initial,
  onSave,
  onClose,
  defaultCurrency,
}: DialogProps<'credit'> & { defaultCurrency?: string }) => {
  const methods = useZodForm({
    schema: creditSchema,
    defaultValues: {
      amount: initial?.amount ?? '',
      currency: initial?.currency ?? defaultCurrency ?? '',
      scope: initial?.scope ?? 'customer',
    },
  })
  return (
    <Form {...methods}>
      <GrantDialogShell
        kind="credit"
        editing={!!initial}
        onClose={onClose}
        onSubmit={methods.handleSubmit(values => {
          onSave({ kind: 'credit', ...values })
          onClose()
        })}
      >
        <div className="grid grid-cols-[1fr_8rem] items-start gap-3">
          <InputFormField
            control={methods.control}
            name="amount"
            label="Amount"
            type="text"
            inputMode="decimal"
            placeholder="0.00"
          />
          <CurrencySelect control={methods.control} name="currency" layout="vertical" />
        </div>
        <ChoiceCardsFormField
          control={methods.control}
          name="scope"
          label="Pays for"
          options={SCOPE_OPTIONS}
        />
      </GrantDialogShell>
    </Form>
  )
}

// ── Discount ──────────────────────────────────────────────────────────────────

const amountRow = z.object({
  currency: z.string().min(1, 'Select a currency'),
  amount: z.string().trim().refine(isPositive, { message: 'Enter a positive amount' }),
})

const optionalNumber = z.preprocess(
  value => (value === '' || value === null ? undefined : value),
  z.coerce.number().optional()
)

const discountSchema = z
  .object({
    discountType: z.enum(['percentage', 'fixed', 'credit']),
    percentage: optionalNumber,
    amount: optionalNumber,
    currency: z.string().optional(),
    otherAmounts: z.array(amountRow),
    addOnIds: z.array(z.string()),
  })
  .superRefine((value, ctx) => {
    if (value.discountType === 'percentage') {
      const pct = value.percentage
      if (pct === undefined || !(pct > 0) || pct > 100) {
        ctx.addIssue({ code: 'custom', message: 'From 0 to 100', path: ['percentage'] })
      }
    } else if (value.discountType === 'fixed') {
      if (value.amount === undefined || !(value.amount > 0)) {
        ctx.addIssue({ code: 'custom', message: 'Enter a positive amount', path: ['amount'] })
      }
      if (!value.currency) {
        ctx.addIssue({ code: 'custom', message: 'Select a currency', path: ['currency'] })
      }
    } else {
      ctx.addIssue({ code: 'custom', message: 'Add credit instead', path: ['discountType'] })
    }
  })

export const DiscountGrantDialog = ({
  initial,
  onSave,
  onClose,
  defaultCurrency,
}: DialogProps<'discount'> & { defaultCurrency?: string }) => {
  const d = initial?.discount
  const methods = useZodForm({
    schema: discountSchema,
    defaultValues: {
      discountType: d?.type ?? 'percentage',
      percentage: d?.type === 'percentage' ? Number(d.percentage) : undefined,
      amount: d?.type === 'fixed' ? Number(d.amount) : undefined,
      currency: d?.type === 'fixed' ? d.currency : defaultCurrency,
      otherAmounts: d?.type === 'fixed' ? d.otherAmounts : [],
      addOnIds: initial?.addOnIds ?? [],
    },
  })
  const discountType = methods.watch('discountType')
  const addOns = useQuery(listAddOns, { pagination: { perPage: 100, page: 0 } }).data?.addOns ?? []
  const couponControl = methods.control as unknown as Control<CouponTermsFormFields>

  return (
    <Form {...methods}>
      <GrantDialogShell
        kind="discount"
        editing={!!initial}
        onClose={onClose}
        onSubmit={methods.handleSubmit(values => {
          onSave({
            kind: 'discount',
            discount:
              values.discountType === 'fixed'
                ? {
                    type: 'fixed',
                    currency: values.currency ?? '',
                    amount: String(values.amount ?? ''),
                    otherAmounts: values.otherAmounts,
                  }
                : { type: 'percentage', percentage: String(values.percentage ?? '') },
            addOnIds: values.addOnIds,
          })
          onClose()
        })}
      >
        <CouponDiscountField control={couponControl} locked={['credit']} />
        {discountType === 'fixed' && <OtherAmountsField control={couponControl} />}
        {addOns.length > 0 && (
          <MultiSelectFormField
            name="addOnIds"
            label="Applies to"
            control={methods.control}
            placeholder="The whole invoice"
            hasSearch
          >
            {addOns.map(addOn => (
              <MultiSelectItem key={addOn.id} value={addOn.id}>
                {addOn.name}
              </MultiSelectItem>
            ))}
          </MultiSelectFormField>
        )}
      </GrantDialogShell>
    </Form>
  )
}

// ── Feature ───────────────────────────────────────────────────────────────────

const featureSchema = z.object({
  featureId: z.string().min(1, 'Pick a feature'),
  mode: z.enum(['on', 'off', 'amount', 'unlimited']),
  amount: z.string().optional(),
})

const freshValue = (feature: Feature | undefined): Pick<RowForm, 'mode' | 'amount'> =>
  feature?.kind === FeatureKind.LIMIT
    ? { mode: 'amount', amount: '' }
    : {
        mode: 'on',
        amount: feature?.value?.type === FeatureValueType.SELECT ? feature.value.options[0] : '',
      }

export const FeatureGrantDialog = ({
  initial,
  onSave,
  onClose,
  excludeFeatureIds,
}: DialogProps<'entitlement'> & { excludeFeatureIds?: ReadonlySet<string> }) => {
  const loaded = useFeatures(initial ? [initial.featureId] : [])
  const [picked, setPicked] = useState<Feature>()
  const [creating, setCreating] = useState(false)
  const feature = picked ?? (initial ? loaded.get(initial.featureId) : undefined)
  const methods = useZodForm({
    schema: featureSchema,
    defaultValues: {
      featureId: initial?.featureId ?? '',
      ...(initial ? valueToForm(initial.value) : { mode: 'on' as const, amount: '' }),
    },
  })

  const pick = (next: Feature | undefined) => {
    if (next?.id === feature?.id) return
    setPicked(next)
    const fresh = freshValue(next)
    methods.setValue('mode', fresh.mode)
    methods.setValue('amount', fresh.amount)
  }

  return (
    <>
      <Form {...methods}>
        <GrantDialogShell
          kind="entitlement"
          editing={!!initial}
          onClose={onClose}
          onSubmit={methods.handleSubmit(values => {
            if (!feature) return
            const form = values as RowForm
            const issue = valueIssue(feature, form)
            if (issue) return methods.setError('amount', { message: issue })
            onSave({
              kind: 'entitlement',
              featureId: feature.id,
              value: create(EntitlementValueSchema, valueFromForm(feature, form)),
              effect: initial?.effect ?? EntitlementEffect.UNSPECIFIED,
            })
            onClose()
          })}
        >
          {initial ? (
            <div className="flex flex-col gap-0.5">
              <span className="text-sm font-medium">{feature?.name ?? '…'}</span>
              {feature?.code && (
                <span className="font-mono text-xs text-muted-foreground">{feature.code}</span>
              )}
            </div>
          ) : (
            <FeaturePicker
              onPick={pick}
              onCreate={() => setCreating(true)}
              exclude={excludeFeatureIds}
            />
          )}
          {feature && <ValueField feature={feature} />}
        </GrantDialogShell>
      </Form>
      {creating && (
        <FeatureCreateSheet
          onClose={(created?: Feature) => {
            setCreating(false)
            if (created) {
              methods.setValue('featureId', created.id, { shouldValidate: true })
              pick(created)
            }
          }}
        />
      )}
    </>
  )
}

// ── Add-on ────────────────────────────────────────────────────────────────────

const addOnSchema = z.object({
  addOnId: z.string().min(1, 'Pick an add-on'),
  quantity: z.coerce
    .number({ error: 'Enter a quantity' })
    .int('Enter a whole number')
    .min(1, 'At least 1')
    .max(10000, 'At most 10000'),
  atEnd: z.enum(['revert', 'convert']),
})

const AT_END_OPTIONS: ChoiceCardOption<'revert' | 'convert'>[] = [
  { value: 'revert', label: 'Removed' },
  { value: 'convert', label: 'Kept', hint: 'At its catalog price' },
]

export const AddOnGrantDialog = ({ initial, onSave, onClose }: DialogProps<'addOn'>) => {
  const methods = useZodForm({
    schema: addOnSchema,
    defaultValues: {
      addOnId: initial?.addOnId ?? '',
      quantity: initial?.quantity ?? 1,
      atEnd: initial?.atEnd ?? 'revert',
    },
  })
  const addOns = useQuery(listAddOns, { pagination: { perPage: 100, page: 0 } }).data?.addOns ?? []

  return (
    <Form {...methods}>
      <GrantDialogShell
        kind="addOn"
        editing={!!initial}
        onClose={onClose}
        onSubmit={methods.handleSubmit(values => {
          onSave({ kind: 'addOn', ...values })
          onClose()
        })}
      >
        <div className="grid grid-cols-[1fr_7rem] items-start gap-3">
          <SelectFormField
            control={methods.control}
            name="addOnId"
            label="Add-on"
            placeholder={addOns.length ? 'Pick an add-on' : 'No add-on'}
          >
            {addOns.map(addOn => (
              <SelectItem key={addOn.id} value={addOn.id}>
                {addOn.name}
              </SelectItem>
            ))}
          </SelectFormField>
          <InputFormField
            control={methods.control}
            name="quantity"
            label="Quantity"
            type="number"
            inputMode="numeric"
            min={1}
            step={1}
          />
        </div>
        <ChoiceCardsFormField
          control={methods.control}
          name="atEnd"
          label="At the end"
          options={AT_END_OPTIONS}
        />
      </GrantDialogShell>
    </Form>
  )
}
