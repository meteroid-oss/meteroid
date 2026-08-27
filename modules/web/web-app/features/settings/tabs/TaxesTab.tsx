import { create } from '@bufbuild/protobuf';
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
import {
  Button,
  Card,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
  Popover,
  PopoverContent,
  PopoverTrigger,
  SelectFormField,
  SelectItem,
  Separator,
  SwitchFormField,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { EditIcon, InfoIcon, PlusIcon, Trash2Icon } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { Control } from 'react-hook-form'
import { toast } from 'sonner'
import { match } from 'ts-pattern'
import { z } from 'zod'

import { CountrySelect } from '@/components/CountrySelect'
import { Loading } from '@/components/Loading'
import { SubdivisionSelect } from '@/components/SubdivisionSelect'
import { InvoicingEntitySelect } from '@/features/settings/components/InvoicingEntitySelect'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  listInvoicingEntities,
  updateInvoicingEntity,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { TaxResolver } from '@/rpc/api/invoicingentities/v1/models_pb'
import { TaxRateSchema, TaxRateNewSchema, TaxRateRuleSchema } from '@/rpc/api/taxes/v1/models_pb';
import {
  createTaxRate,
  createTaxCategory,
  deleteTaxRate,
  deleteTaxCategory,
  listTaxRates,
  listTaxCategories,
  updateTaxRate,
  updateTaxCategory,
} from '@/rpc/api/taxes/v1/taxes-TaxesService_connectquery'

import type { TaxRate, TaxCategory } from '@/rpc/api/taxes/v1/models_pb';

// Radix Select forbids empty-string item values, so the optional "none" choices
// (category parent, entity default) use a sentinel that maps to '' at the API.
const NO_CATEGORY = '__none__'

// EU seller allowlist — mirrors the backend `world_tax::EU_SELLER_COUNTRY_CODES`.
// The built-in Meteroid EU VAT engine is EU-seller-only; keep this list in one
// place so adding an explicitly-supported near-EU seller later is a one-line change.
const EU_SELLER_COUNTRY_CODES = new Set([
  'AT', 'BE', 'BG', 'CY', 'CZ', 'DE', 'DK', 'EE', 'ES', 'FI', 'FR', 'GR', 'HR', 'HU', 'IE', 'IT',
  'LT', 'LU', 'LV', 'MT', 'NL', 'PL', 'PT', 'RO', 'SE', 'SI', 'SK',
])

const isEuSellerCountry = (code?: string) =>
  !!code && EU_SELLER_COUNTRY_CODES.has(code.toUpperCase())

const taxSettingsSchema = z.object({
  taxResolver: z.enum(['NONE', 'MANUAL', 'METEROID_EU_VAT']).optional(),
  requireViesValidForReverseCharge: z.boolean().optional(),
  defaultTaxCategoryId: z.string().optional(),
})

const taxCategorySchema = z.object({
  name: z.string().min(1, 'Name is required'),
  parentId: z.string().optional(),
})

// A custom rate is an override: it targets a product tax category and defines a
// rate per customer jurisdiction. Scope is category-only (no per-product links).
const customRateSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  taxCategoryId: z.string().min(1, 'Choose which product category this applies to'),
  accountingCode: z.string().min(1, 'Accounting code is required'),
  rules: z
    .array(
      z.object({
        country: z.string().optional(),
        region: z.string().optional(),
        rate: z.string().regex(/^\d+(\.\d+)?$/, 'Rate must be a valid decimal'),
      })
    )
    .min(1, 'Add at least one rate'),
})

// Rates are stored as fractions (0.2 = 20%); the form and the table speak percent.
const rateToPercent = (rate: string) => {
  const n = Number(rate)
  return Number.isFinite(n) ? String(Number((n * 100).toFixed(6))) : rate
}

const percentToRate = (percent: string) => {
  const n = Number(percent)
  return Number.isFinite(n) ? String(Number((n / 100).toFixed(8))) : percent
}

const RateRuleRow = ({
  index,
  control,
  onRemove,
  showRemove,
}: {
  index: number
  control: Control<z.infer<typeof customRateSchema>>
  onRemove: (index: number) => void
  showRemove: boolean
}) => {
  return (
    <div className="flex gap-2">
      <CountrySelect
        name={`rules.${index}.country`}
        placeholder="Customer country (any)"
        control={control}
        className="flex-1"
        label=""
        clearable
      />
      <SubdivisionSelect
        name={`rules.${index}.region`}
        countryFieldName={`rules.${index}.country`}
        placeholder="Region (any)"
        control={control}
        className="flex-1"
        label=""
        clearable
      />
      <InputFormField
        name={`rules.${index}.rate`}
        placeholder="Rate %"
        control={control}
        containerClassName="w-24"
      />
      {showRemove && (
        <Button type="button" size="icon" variant="ghost" onClick={() => onRemove(index)}>
          <Trash2Icon className="h-4 w-4" />
        </Button>
      )}
    </div>
  )
}

// Small (i) trigger that tucks the fuller explanation of a section into a popover,
// keeping the section heading to a short one-liner.
const InfoHint = ({ label, children }: { label: string; children: ReactNode }) => (
  <Popover>
    <PopoverTrigger asChild>
      <button
        type="button"
        aria-label={label}
        className="text-muted-foreground hover:text-foreground transition-colors"
      >
        <InfoIcon className="h-3.5 w-3.5" />
      </button>
    </PopoverTrigger>
    <PopoverContent align="start" className="w-80 text-xs text-muted-foreground leading-relaxed">
      {children}
    </PopoverContent>
  </Popover>
)

// The full resolution ladder, relocated from a standalone card into an info
// popover next to the Method selector so the page stays compact.
const TaxDecisionPopover = ({
  isEuVat,
  isNone,
}: {
  isEuVat: boolean
  isNone: boolean
}) => {
  const steps = [
    {
      n: 1,
      title: 'Customer is exempt or reverse-charged',
      body: 'The customer’s tax status wins first — a tax-exempt customer is untaxed, and intra-EU B2B reverse charge shifts the liability to them (validated by VIES when required).',
    },
    {
      n: 2,
      title: 'Product category is Non-taxable',
      body: 'Lines whose product category is Non-taxable carry no tax, whatever the method or custom rates say.',
    },
    {
      n: 3,
      title: 'A custom rate matches the category and country',
      body: isEuVat
        ? 'A custom rate targeting the line’s category, with a rule matching the customer’s destination country, replaces the calculated VAT for that line.'
        : 'A custom rate targeting the line’s category, with a rule matching the customer’s destination country, sets the tax for that line.',
    },
    {
      n: 4,
      title: 'Otherwise, the method decides',
      body: 'Manual — the customer’s own flat rate applies, otherwise the line is untaxed. Automatic EU VAT — the engine computes VAT by scenario (your domestic rate, reverse charge for intra-EU B2B, the customer’s country rate for intra-EU B2C, zero-rating for exports).',
    },
  ]

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          <InfoIcon className="h-3.5 w-3.5" />
          How is tax decided?
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-96 space-y-3">
        <div>
          <div className="text-sm font-medium">How tax is decided</div>
          <p className="text-xs text-muted-foreground mt-1">
            The method is the gate. When it is <span className="font-medium">No tax</span>, nothing
            is taxed. Otherwise each invoice line is resolved in order — first match wins.
          </p>
        </div>
        {isNone ? (
          <div className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
            This entity is set to <span className="font-medium">No tax</span>, so the steps below do
            not run. Switch to Manual or Automatic EU VAT to enable them.
          </div>
        ) : (
          <ol className="space-y-2">
            {steps.map(step => (
              <li key={step.n} className="flex gap-2.5">
                <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border border-border text-[10px] font-medium text-muted-foreground">
                  {step.n}
                </span>
                <div>
                  <div className="text-xs font-medium">{step.title}</div>
                  <div className="text-xs text-muted-foreground">{step.body}</div>
                </div>
              </li>
            ))}
          </ol>
        )}
      </PopoverContent>
    </Popover>
  )
}

export const TaxesTab = () => {
  const queryClient = useQueryClient()
  const { selectedEntityId: invoiceEntityId, entities, isLoading } = useInvoicingEntity()

  const [rateDialogOpen, setRateDialogOpen] = useState(false)
  const [editingRate, setEditingRate] = useState<TaxRate | null>(null)
  const [categoryDialogOpen, setCategoryDialogOpen] = useState(false)
  const [editingCategory, setEditingCategory] = useState<TaxCategory | null>(null)

  const invalidateTaxCategories = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listTaxCategories,
        input: {},
        cardinality: 'finite',
      }),
    })

  const invalidateTaxRates = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listTaxRates,
        input: { invoicingEntityId: invoiceEntityId ?? '' },
        cardinality: 'finite',
      }),
    })

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntity, {
    onSuccess: async res => {
      if (res.entity) {
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: listInvoicingEntities,
            cardinality: undefined,
          }),
        })
        toast.success('Tax settings updated')
      }
    },
  })

  const listTaxRatesQuery = useQuery(
    listTaxRates,
    { invoicingEntityId: invoiceEntityId ?? '' },
    { enabled: !!invoiceEntityId }
  )

  const listTaxCategoriesQuery = useQuery(listTaxCategories, {})
  const categories = listTaxCategoriesQuery.data?.taxCategories ?? []

  const categoryNameById = new Map(categories.map(c => [c.id, c.name]))

  const methods = useZodForm({
    schema: taxSettingsSchema,
  })

  const rateMethods = useZodForm({
    schema: customRateSchema,
    mode: 'onChange',
    defaultValues: {
      name: '',
      taxCategoryId: '',
      accountingCode: '',
      rules: [{ country: '', region: '', rate: '' }],
    },
  })

  const createRateMut = useMutation(createTaxRate, {
    onSuccess: async () => {
      await invalidateTaxRates()
      toast.success('Custom rate created')
      setRateDialogOpen(false)
      setEditingRate(null)
    },
    onError: error => toast.error(`Failed to create custom rate: ${error.message}`),
  })

  const updateRateMut = useMutation(updateTaxRate, {
    onSuccess: async () => {
      await invalidateTaxRates()
      toast.success('Custom rate updated')
      setRateDialogOpen(false)
      setEditingRate(null)
    },
    onError: error => toast.error(`Failed to update custom rate: ${error.message}`),
  })

  const deleteRateMut = useMutation(deleteTaxRate, {
    onSuccess: async () => {
      await invalidateTaxRates()
      toast.success('Custom rate deleted')
    },
    onError: error => toast.error(`Failed to delete custom rate: ${error.message}`),
  })

  const categoryMethods = useZodForm({
    schema: taxCategorySchema,
    mode: 'onChange',
    defaultValues: { name: '', parentId: NO_CATEGORY },
  })

  const createTaxCategoryMut = useMutation(createTaxCategory, {
    onSuccess: async () => {
      await invalidateTaxCategories()
      toast.success('Tax category created')
      setCategoryDialogOpen(false)
      setEditingCategory(null)
    },
    onError: error => toast.error(`Failed to create tax category: ${error.message}`),
  })

  const updateTaxCategoryMut = useMutation(updateTaxCategory, {
    onSuccess: async () => {
      await invalidateTaxCategories()
      toast.success('Tax category updated')
      setCategoryDialogOpen(false)
      setEditingCategory(null)
    },
    onError: error => toast.error(`Failed to update tax category: ${error.message}`),
  })

  const deleteTaxCategoryMut = useMutation(deleteTaxCategory, {
    onSuccess: async () => {
      await invalidateTaxCategories()
      toast.success('Tax category deleted')
    },
    onError: error => toast.error(`Failed to delete tax category: ${error.message}`),
  })

  useEffect(() => {
    const entity = entities.find(entity => entity.id === invoiceEntityId)

    if (entity) {
      methods.setValue(
        'taxResolver',
        match(entity.taxResolver)
          .with(TaxResolver.NONE, () => 'NONE' as const)
          .with(TaxResolver.MANUAL, () => 'MANUAL' as const)
          .with(TaxResolver.METEROID_EU_VAT, () => 'METEROID_EU_VAT' as const)
          .otherwise(() => 'NONE' as const)
      )
      methods.setValue('requireViesValidForReverseCharge', entity.requireViesValidForReverseCharge)
      methods.setValue('defaultTaxCategoryId', entity.defaultTaxCategoryId || NO_CATEGORY)
    } else {
      methods.reset()
    }
  }, [invoiceEntityId, entities])

  const selectedEntity = entities.find(entity => entity.id === invoiceEntityId)
  const euSeller = isEuSellerCountry(selectedEntity?.country)

  // Custom rates are "Overrides" under EU VAT (they replace the computed rate) and
  // plain "Rates" under Manual (there is nothing to override — they define the tax).
  // Drive the method-dependent UI (help text, Overrides/Rates naming, and hiding
  // the rest under "No tax") from the LIVE selection so it reacts immediately,
  // not only after the settings are saved.
  const watchedResolver = methods.watch('taxResolver') ?? selectedEntity?.taxResolver
  const isEuVatResolver =
    watchedResolver === 'METEROID_EU_VAT' || watchedResolver === TaxResolver.METEROID_EU_VAT
  const isNoneResolver = watchedResolver === 'NONE' || watchedResolver === TaxResolver.NONE

  // Contextual one-liner shown under the Method selector — only what applies to the
  // currently-saved method. The fuller ladder lives in the (i) popover beside it.
  const methodHelp = isNoneResolver
    ? 'No tax on any line — custom rates and customer status are ignored.'
    : isEuVatResolver
      ? 'VAT is computed per line by scenario; customer status and overrides take precedence.'
      : 'Each line uses the customer’s flat rate, or a matching custom rate — otherwise untaxed.'

  const customRatesTitle = isEuVatResolver ? 'Overrides' : 'Custom rates'
  // Short one-liner under the heading; the fuller explanation lives in the (i) hint.
  const customRatesOneLiner = isEuVatResolver
    ? 'Replace the calculated VAT for a product category, by customer location.'
    : 'Set the tax for a product category, by customer location.'
  const customRatesDescription = isEuVatResolver
    ? 'Replace the calculated VAT for a product category — for example a specific country’s statutory rate. Matched to your customer’s location and applied after tax-exempt / reverse-charge customers, replacing the calculated VAT for that line.'
    : 'Define the tax for a product category by your customer’s location — for example a specific country’s rate. There is no calculated VAT under manual tax, so these rates set the tax directly (after tax-exempt / reverse-charge customers).'
  const rateDialogNoun = isEuVatResolver ? 'override' : 'rate'
  const rateDialogDescription = isEuVatResolver
    ? 'A named rate that replaces the calculated VAT for a product category, chosen by your customer’s location.'
    : 'A named rate that defines the tax for a product category, chosen by your customer’s location.'

  if (isLoading) {
    return <Loading />
  }

  const onSubmit = async (values: z.infer<typeof taxSettingsSchema>) => {
    await updateInvoicingEntityMut.mutateAsync({
      id: invoiceEntityId,
      data: {
        taxResolver: match(values.taxResolver)
          .with('NONE', () => TaxResolver.NONE)
          .with('MANUAL', () => TaxResolver.MANUAL)
          .with('METEROID_EU_VAT', () => TaxResolver.METEROID_EU_VAT)
          .otherwise(() => TaxResolver.NONE),
        requireViesValidForReverseCharge: values.requireViesValidForReverseCharge,
        defaultTaxCategoryId:
          values.defaultTaxCategoryId && values.defaultTaxCategoryId !== NO_CATEGORY
            ? values.defaultTaxCategoryId
            : '',
      },
    })
  }

  const openCreateRate = () => {
    setEditingRate(null)
    rateMethods.reset({
      name: '',
      taxCategoryId: '',
      accountingCode: '',
      rules: [{ country: '', region: '', rate: '' }],
    })
    setRateDialogOpen(true)
  }

  const handleEditRate = (rate: TaxRate) => {
    setEditingRate(rate)
    rateMethods.reset({
      name: rate.name,
      taxCategoryId: rate.taxCategoryId || '',
      accountingCode: rate.taxCode,
      rules: rate.rules.map(rule => ({
        country: rule.country || '',
        region: rule.region || '',
        rate: rateToPercent(rule.rate),
      })),
    })
    setRateDialogOpen(true)
  }

  const handleDeleteRate = async (rate: TaxRate) => {
    if (confirm(`Delete custom rate "${rate.name}"?`)) {
      await deleteRateMut.mutateAsync({ id: rate.id })
    }
  }

  const onSubmitRate = async (values: z.infer<typeof customRateSchema>) => {
    if (!invoiceEntityId) return

    const rules = values.rules.map(rule =>
      create(TaxRateRuleSchema, {
        country: rule.country || undefined,
        region: rule.region || undefined,
        rate: percentToRate(rule.rate),
      })
    )

    if (editingRate) {
      await updateRateMut.mutateAsync({
        taxRate: create(TaxRateSchema, {
          id: editingRate.id,
          invoicingEntityId: invoiceEntityId,
          name: values.name,
          taxCode: values.accountingCode,
          taxCategoryId: values.taxCategoryId,
          rules,
        }),
      })
    } else {
      await createRateMut.mutateAsync({
        taxRate: create(TaxRateNewSchema, {
          invoicingEntityId: invoiceEntityId,
          name: values.name,
          taxCode: values.accountingCode,
          taxCategoryId: values.taxCategoryId,
          rules,
        }),
      })
    }
  }

  const openCreateCategory = () => {
    setEditingCategory(null)
    categoryMethods.reset({ name: '', parentId: NO_CATEGORY })
    setCategoryDialogOpen(true)
  }

  const handleEditCategory = (cat: TaxCategory) => {
    setEditingCategory(cat)
    categoryMethods.reset({
      name: cat.name,
      parentId: cat.parentId || NO_CATEGORY,
    })
    setCategoryDialogOpen(true)
  }

  const handleDeleteCategory = async (cat: TaxCategory) => {
    if (
      confirm(
        `Delete tax category "${cat.name}"? Products and custom rates referencing it fall back to the default.`
      )
    ) {
      await deleteTaxCategoryMut.mutateAsync({ id: cat.id })
    }
  }

  const onSubmitCategory = async (values: z.infer<typeof taxCategorySchema>) => {
    const parentId =
      values.parentId && values.parentId !== NO_CATEGORY ? values.parentId : undefined
    if (editingCategory) {
      await updateTaxCategoryMut.mutateAsync({
        id: editingCategory.id,
        name: values.name,
        parentId,
      })
    } else {
      await createTaxCategoryMut.mutateAsync({ name: values.name, parentId })
    }
  }

  const handleAddRule = () => {
    rateMethods.setValue('rules', [
      ...rateMethods.getValues('rules'),
      { country: '', region: '', rate: '' },
    ])
  }

  const handleRemoveRule = (index: number) => {
    const current = rateMethods.getValues('rules')
    if (current.length > 1) {
      rateMethods.setValue('rules', current.filter((_, i) => i !== index))
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <Card className="px-8 py-6 max-w-[950px] space-y-5">
        {/* Card header — the entity selector scopes every sub-section below */}
        <div className="flex items-center justify-between gap-4">
          <div>
            <h3 className="font-medium text-sm">Tax calculation</h3>
            <p className="text-xs text-muted-foreground mt-0.5">
              How tax is calculated for this invoicing entity&apos;s invoices.
            </p>
          </div>
          <InvoicingEntitySelect />
        </div>

        {/* Sub-section: method + defaults */}
        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-3">
            <div className="grid grid-cols-6 gap-4">
              <SelectFormField
                name="taxResolver"
                control={methods.control}
                label="Method"
                placeholder="Select how tax is calculated"
                containerClassName="col-span-6"
                className="max-w-md"
              >
                <SelectItem value="NONE">No tax</SelectItem>
                <SelectItem value="MANUAL">Manual — custom rates only</SelectItem>
                <SelectItem value="METEROID_EU_VAT" disabled={!euSeller}>
                  Automatic EU VAT
                </SelectItem>
              </SelectFormField>
              <div className="col-span-6 -mt-2 space-y-1">
                <p className="text-xs text-muted-foreground">
                  {methodHelp}
                  {!euSeller &&
                    ' Automatic EU VAT is available to EU-based sellers only — non-EU sellers use manual rates.'}
                </p>
                <TaxDecisionPopover isEuVat={isEuVatResolver} isNone={isNoneResolver} />
              </div>

              {!isNoneResolver && (
                <>
                  <SelectFormField
                    name="defaultTaxCategoryId"
                    control={methods.control}
                    label="Default product tax category"
                    placeholder="No default"
                    containerClassName="col-span-6"
                    className="max-w-md"
                  >
                    <SelectItem value={NO_CATEGORY}>No default</SelectItem>
                    {categories.map(c => (
                      <SelectItem key={c.id} value={c.id}>
                        {c.name}
                      </SelectItem>
                    ))}
                  </SelectFormField>
                  <p className="text-xs text-muted-foreground col-span-6 -mt-2">
                    Used for invoice lines whose product has no tax category set.
                  </p>
                  <SwitchFormField
                    name="requireViesValidForReverseCharge"
                    control={methods.control}
                    label="Require a valid tax ID for reverse charge"
                    description="Apply reverse charge only after the customer's VAT number passes VIES; until then their country's standard rate applies."
                    containerClassName="col-span-6"
                  />
                </>
              )}
            </div>

            <div className="pt-2 flex justify-end items-center">
              <Button
                size="sm"
                disabled={
                  !methods.formState.isValid ||
                  !methods.formState.isDirty ||
                  updateInvoicingEntityMut.isPending
                }
              >
                Save changes
              </Button>
            </div>
          </form>
        </Form>

        {invoiceEntityId && !isNoneResolver && (
          <>
            <Separator />

            {/* Sub-section: custom rates / overrides */}
            <div className="space-y-3">
              <div className="flex justify-between items-start gap-4">
                <div>
                  <div className="flex items-center gap-1.5">
                    <h4 className="font-medium text-sm">{customRatesTitle}</h4>
                    <InfoHint label={`About ${customRatesTitle.toLowerCase()}`}>
                      {customRatesDescription}
                    </InfoHint>
                  </div>
                  <p className="text-xs text-muted-foreground mt-0.5">{customRatesOneLiner}</p>
                </div>
                <Button size="sm" className="shrink-0" variant="outline" onClick={openCreateRate}>
                  <PlusIcon className="h-4 w-4 mr-2" />
                  Add {rateDialogNoun}
                </Button>
              </div>

              {listTaxRatesQuery.isLoading ? (
                <Loading />
              ) : listTaxRatesQuery.data?.taxRates && listTaxRatesQuery.data.taxRates.length > 0 ? (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Name</TableHead>
                      <TableHead>Applies to</TableHead>
                      <TableHead>Rate by customer location</TableHead>
                      <TableHead>Accounting code</TableHead>
                      <TableHead className="text-right w-[90px]">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {listTaxRatesQuery.data.taxRates.map(rate => (
                      <TableRow key={rate.id}>
                        <TableCell className="font-medium">{rate.name}</TableCell>
                        <TableCell className="text-sm">
                          {categoryNameById.get(rate.taxCategoryId ?? '') ?? (
                            <span className="text-muted-foreground">—</span>
                          )}
                        </TableCell>
                        <TableCell>
                          <div className="space-y-1">
                            {rate.rules.map((rule, idx) => (
                              <div key={idx} className="text-sm">
                                {rule.country || 'Any country'}
                                {rule.region && ` · ${rule.region}`}: {rateToPercent(rule.rate)}%
                              </div>
                            ))}
                          </div>
                        </TableCell>
                        <TableCell className="text-sm text-muted-foreground">
                          <code className="text-xs">{rate.taxCode}</code>
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex justify-end gap-2">
                            <Button
                              size="icon"
                              variant="ghost"
                              onClick={() => handleEditRate(rate)}
                            >
                              <EditIcon className="h-4 w-4" />
                            </Button>
                            <Button
                              size="icon"
                              variant="ghost"
                              onClick={() => handleDeleteRate(rate)}
                            >
                              <Trash2Icon className="h-4 w-4" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              ) : (
                <div className="text-center py-6 text-sm text-muted-foreground">
                  No custom rates yet.
                </div>
              )}
            </div>

            <Separator />

            {/* Sub-section: product tax categories */}
            <div className="space-y-3">
              <div className="flex justify-between items-start gap-4">
                <div>
                  <div className="flex items-center gap-1.5">
                    <h4 className="font-medium text-sm">Product tax categories</h4>
                    <InfoHint label="About product tax categories">
                      Categories are standard-rated by default; a custom rate above (or an external
                      provider) can target a category, and Non-taxable is special-cased. Products
                      without a category use the default set above.
                    </InfoHint>
                  </div>
                  <p className="text-xs text-muted-foreground mt-0.5">
                    Classify what your products are for tax purposes.
                  </p>
                </div>
                <Button
                  size="sm"
                  className="shrink-0"
                  variant="outline"
                  onClick={openCreateCategory}
                >
                  <PlusIcon className="h-4 w-4 mr-2" />
                  Add category
                </Button>
              </div>

              {listTaxCategoriesQuery.isLoading ? (
                <Loading />
              ) : categories.length ? (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Name</TableHead>
                      <TableHead>Key</TableHead>
                      <TableHead>Source</TableHead>
                      <TableHead className="text-right w-[90px]">Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {categories.map(cat => (
                      <TableRow key={cat.id}>
                        <TableCell className="font-medium">
                          {cat.name}
                          {selectedEntity?.defaultTaxCategoryId === cat.id && (
                            <span className="ml-2 text-xs font-normal text-muted-foreground rounded border border-border px-1.5 py-0.5">
                              Default
                            </span>
                          )}
                        </TableCell>
                        <TableCell>
                            <code className="text-xs">{cat.key}</code>
                        </TableCell>
                        <TableCell className="text-muted-foreground text-sm">
                          {cat.isBuiltin ? 'Built-in' : 'Custom'}
                        </TableCell>
                        <TableCell className="text-right">
                          {cat.isBuiltin ? (
                            <span className="text-xs text-muted-foreground">—</span>
                          ) : (
                            <div className="flex justify-end gap-2">
                              <Button
                                size="icon"
                                variant="ghost"
                                onClick={() => handleEditCategory(cat)}
                              >
                                <EditIcon className="h-4 w-4" />
                              </Button>
                              <Button
                                size="icon"
                                variant="ghost"
                                onClick={() => handleDeleteCategory(cat)}
                              >
                                <Trash2Icon className="h-4 w-4" />
                              </Button>
                            </div>
                          )}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              ) : (
                <div className="text-center py-6 text-sm text-muted-foreground">
                  No tax categories available.
                </div>
              )}
            </div>
          </>
        )}
      </Card>

      {/* Custom rate dialog */}
      <Dialog open={rateDialogOpen} onOpenChange={setRateDialogOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>
              {editingRate ? `Edit ${rateDialogNoun}` : `Add ${rateDialogNoun}`}
            </DialogTitle>
            <DialogDescription>{rateDialogDescription}</DialogDescription>
          </DialogHeader>

          <Form {...rateMethods}>
            <form onSubmit={rateMethods.handleSubmit(onSubmitRate)} className="space-y-4">
              <InputFormField
                name="name"
                label="Name"
                placeholder="e.g., French VAT, US sales tax"
                control={rateMethods.control}
              />

              <div className="space-y-1">
                <SelectFormField
                  name="taxCategoryId"
                  control={rateMethods.control}
                  label="Applies to"
                  placeholder="Choose a product category"
                  containerClassName="w-full"
                >
                  {categories.map(c => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.name}
                    </SelectItem>
                  ))}
                </SelectFormField>
                <p className="text-xs text-muted-foreground">
                  Applies to every invoice line whose product is in this category.
                </p>
              </div>

              <div className="space-y-2 pt-4">
                <label className="text-sm font-medium">Rate by customer location</label>
                <p className="text-xs text-muted-foreground">
                  Matched to the customer&apos;s country/region; the most specific match wins. Leave
                  the country empty for a catch-all rate.
                </p>
                {rateMethods.watch('rules').map((_, index) => (
                  <RateRuleRow
                    key={index}
                    index={index}
                    control={rateMethods.control}
                    onRemove={handleRemoveRule}
                    showRemove={rateMethods.watch('rules').length > 1}
                  />
                ))}
                <Button type="button" size="sm" variant="outline" onClick={handleAddRule}>
                  <PlusIcon className="h-4 w-4 mr-2" />
                  Add rate
                </Button>
              </div>

              <InputFormField
                name="accountingCode"
                label="Accounting code"
                placeholder="e.g., VAT_FR, SALES_US"
                control={rateMethods.control}
              />

              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setRateDialogOpen(false)
                    setEditingRate(null)
                  }}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  disabled={
                    createRateMut.isPending ||
                    updateRateMut.isPending ||
                    !rateMethods.formState.isValid
                  }
                >
                  {editingRate ? 'Update' : 'Create'}
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>

      {/* Tax category dialog */}
      <Dialog open={categoryDialogOpen} onOpenChange={setCategoryDialogOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>
              {editingCategory ? 'Edit tax category' : 'Create tax category'}
            </DialogTitle>
            <DialogDescription>
              Classifies what a product is for tax purposes. Standard-rated unless a custom rate
              targets it.
            </DialogDescription>
          </DialogHeader>

          <Form {...categoryMethods}>
            <form onSubmit={categoryMethods.handleSubmit(onSubmitCategory)} className="space-y-4">
              <InputFormField
                name="name"
                label="Name"
                placeholder="e.g., Streaming services"
                control={categoryMethods.control}
              />

              <SelectFormField
                name="parentId"
                control={categoryMethods.control}
                label="Parent category (optional)"
                placeholder="None"
                containerClassName="w-full"
              >
                <SelectItem value={NO_CATEGORY}>None</SelectItem>
                {categories
                  .filter(c => c.id !== editingCategory?.id)
                  .map(c => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.name}
                    </SelectItem>
                  ))}
              </SelectFormField>

              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setCategoryDialogOpen(false)
                    setEditingCategory(null)
                  }}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  disabled={
                    createTaxCategoryMut.isPending ||
                    updateTaxCategoryMut.isPending ||
                    !categoryMethods.formState.isValid
                  }
                >
                  {editingCategory ? 'Update' : 'Create'}
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>
    </div>
  )
}
