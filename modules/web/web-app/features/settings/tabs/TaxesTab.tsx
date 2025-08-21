import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import {
  Badge,
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
  SelectFormField,
  SelectItem,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { EditIcon, PlusIcon, Trash2Icon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { match } from 'ts-pattern'
import { z } from 'zod'

import { Combobox } from '@/components/Combobox'
import { Loading } from '@/components/Loading'
import { CreateInvoicingEntityDialog } from '@/features/settings/CreateInvoiceEntityDialog'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  listInvoicingEntities,
  updateInvoicingEntity,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { TaxResolver } from '@/rpc/api/invoicingentities/v1/models_pb'
import { CustomTax, CustomTaxNew, TaxRule } from '@/rpc/api/taxes/v1/models_pb'
import {
  createCustomTax,
  deleteCustomTax,
  listCustomTaxes,
  updateCustomTax,
} from '@/rpc/api/taxes/v1/taxes-TaxesService_connectquery'

const taxSettingsSchema = z.object({
  taxResolver: z.enum(['NONE', 'MANUAL', 'METEROID_EU_VAT']).optional(),
})

const customTaxSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  taxCode: z.string().min(1, 'Tax code is required'),
  rules: z
    .array(
      z.object({
        country: z.string().optional(),
        region: z.string().optional(),
        rate: z.string().regex(/^\d+(\.\d+)?$/, 'Rate must be a valid decimal'),
      })
    )
    .min(1, 'At least one tax rule is required'),
})

export const TaxesTab = () => {
  const listInvoicingEntitiesQuery = useQuery(listInvoicingEntities)
  const queryClient = useQueryClient()

  const [createDialogOpen, setCreateDialogOpen] = useState(false)
  const [customTaxDialogOpen, setCustomTaxDialogOpen] = useState(false)
  const [editingCustomTax, setEditingCustomTax] = useState<CustomTax | null>(null)

  const updateInvoicingEntityMut = useMutation(updateInvoicingEntity, {
    onSuccess: async res => {
      if (res.entity) {
        queryClient.setQueryData(
          createConnectQueryKey(listInvoicingEntities),
          createProtobufSafeUpdater(listInvoicingEntities, prev => {
            return {
              entities: prev?.entities.map(entity => {
                if (entity.id === res.entity?.id) {
                  return res.entity
                } else {
                  return entity
                }
              }),
            }
          })
        )
        toast.success('Tax settings updated')
      }
    },
  })

  const defaultInvoicingEntity = listInvoicingEntitiesQuery.data?.entities?.find(
    entity => entity.isDefault
  )

  const [invoiceEntityId, setInvoiceEntityId] = useState<string | undefined>(
    defaultInvoicingEntity?.id
  )

  const listCustomTaxesQuery = useQuery(
    listCustomTaxes,
    {
      invoicingEntityId: invoiceEntityId ?? '',
    },
    {
      enabled: !!invoiceEntityId,
    }
  )

  const methods = useZodForm({
    schema: taxSettingsSchema,
  })

  const customTaxMethods = useZodForm({
    schema: customTaxSchema,
    defaultValues: {
      name: '',
      taxCode: '',
      rules: [{ country: '', region: '', rate: '' }],
    },
  })

  const createCustomTaxMut = useMutation(createCustomTax, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listCustomTaxes, {
          invoicingEntityId: invoiceEntityId ?? '',
        }),
      })
      toast.success('Custom tax created successfully')
      setCustomTaxDialogOpen(false)
      customTaxMethods.reset()
    },
    onError: error => {
      toast.error(`Failed to create custom tax: ${error.message}`)
    },
  })

  const updateCustomTaxMut = useMutation(updateCustomTax, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listCustomTaxes, {
          invoicingEntityId: invoiceEntityId ?? '',
        }),
      })
      toast.success('Custom tax updated successfully')
      setCustomTaxDialogOpen(false)
      setEditingCustomTax(null)
      customTaxMethods.reset()
    },
    onError: error => {
      toast.error(`Failed to update custom tax: ${error.message}`)
    },
  })

  const deleteCustomTaxMut = useMutation(deleteCustomTax, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listCustomTaxes, {
          invoicingEntityId: invoiceEntityId ?? '',
        }),
      })
      toast.success('Custom tax deleted successfully')
    },
    onError: error => {
      toast.error(`Failed to delete custom tax: ${error.message}`)
    },
  })

  useEffect(() => {
    const entity = listInvoicingEntitiesQuery.data?.entities?.find(
      entity => entity.id === invoiceEntityId
    )

    if (entity) {
      methods.setValue(
        'taxResolver',
        match(entity.taxResolver)
          .with(TaxResolver.NONE, () => 'NONE' as const)
          .with(TaxResolver.MANUAL, () => 'MANUAL' as const)
          .with(TaxResolver.METEROID_EU_VAT, () => 'METEROID_EU_VAT' as const)
          .otherwise(() => 'NONE' as const)
      )
    } else {
      methods.reset()
    }
  }, [invoiceEntityId])

  useEffect(() => {
    if (defaultInvoicingEntity && !invoiceEntityId) {
      setInvoiceEntityId(defaultInvoicingEntity.id)
    }
  }, [defaultInvoicingEntity])

  if (listInvoicingEntitiesQuery.isLoading) {
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
      },
    })
  }

  const onSubmitCustomTax = async (values: z.infer<typeof customTaxSchema>) => {
    if (!invoiceEntityId) return

    const taxRules = values.rules.map(
      rule =>
        new TaxRule({
          country: rule.country || undefined,
          region: rule.region || undefined,
          rate: rule.rate,
        })
    )

    if (editingCustomTax) {
      await updateCustomTaxMut.mutateAsync({
        customTax: new CustomTax({
          id: editingCustomTax.id,
          invoicingEntityId: invoiceEntityId,
          name: values.name,
          taxCode: values.taxCode,
          rules: taxRules,
        }),
      })
    } else {
      await createCustomTaxMut.mutateAsync({
        customTax: new CustomTaxNew({
          invoicingEntityId: invoiceEntityId,
          name: values.name,
          taxCode: values.taxCode,
          rules: taxRules,
        }),
      })
    }
  }

  const handleEditCustomTax = (tax: CustomTax) => {
    setEditingCustomTax(tax)
    customTaxMethods.reset({
      name: tax.name,
      taxCode: tax.taxCode,
      rules: tax.rules.map(rule => ({
        country: rule.country || '',
        region: rule.region || '',
        rate: rule.rate,
      })),
    })
    setCustomTaxDialogOpen(true)
  }

  const handleDeleteCustomTax = async (taxId: string) => {
    if (confirm('Are you sure you want to delete this custom tax?')) {
      await deleteCustomTaxMut.mutateAsync({ id: taxId })
    }
  }

  const handleAddTaxRule = () => {
    const currentRules = customTaxMethods.getValues('rules')
    customTaxMethods.setValue('rules', [...currentRules, { country: '', region: '', rate: '' }])
  }

  const handleRemoveTaxRule = (index: number) => {
    const currentRules = customTaxMethods.getValues('rules')
    if (currentRules.length > 1) {
      customTaxMethods.setValue(
        'rules',
        currentRules.filter((_, i) => i !== index)
      )
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
          <Card className="px-8 py-6 max-w-[950px] space-y-4">
            <div className="grid grid-cols-6 gap-4">
              <div className="col-span-2">
                <h3 className="font-medium text-lg">Tax Configuration</h3>
                <p className="text-sm text-muted-foreground mt-1">
                  Configure how taxes are calculated and applied to invoices.
                </p>
              </div>
              <div className="col-span-4 content-center flex flex-row">
                <div className="flex-grow"></div>
                <Combobox
                  placeholder="Select"
                  className="max-w-[300px]"
                  value={invoiceEntityId}
                  onChange={setInvoiceEntityId}
                  options={
                    listInvoicingEntitiesQuery.data?.entities.map(entity => ({
                      label: (
                        <div className="flex flex-row w-full">
                          <div className="pr-2">{getCountryFlagEmoji(entity.country)}</div>
                          <div>{entity.legalName}</div>
                          <div className="flex-grow" />
                          {entity.isDefault && (
                            <Badge variant="primary" size="sm">
                              Default
                            </Badge>
                          )}
                        </div>
                      ),
                      value: entity.id,
                    })) ?? []
                  }
                  action={
                    <Button
                      size="content"
                      variant="ghost"
                      hasIcon
                      className="w-full border-none h-full"
                      onClick={() => setCreateDialogOpen(true)}
                    >
                      <PlusIcon size="12" /> New invoicing entity
                    </Button>
                  }
                />
              </div>
            </div>

            <div className="grid grid-cols-6 gap-4 pt-1">
              <SelectFormField
                name="taxResolver"
                control={methods.control}
                label="Tax Calculation Method"
                placeholder="Select how taxes should be calculated"
                containerClassName="col-span-6"
              >
                <SelectItem value="NONE">None</SelectItem>
                <SelectItem value="MANUAL">Manual</SelectItem>
                <SelectItem value="METEROID_EU_VAT">Meteroid EU VAT</SelectItem>
              </SelectFormField>
            </div>

            <div className="pt-10 flex justify-end items-center">
              <div>
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
            </div>
          </Card>
        </form>
      </Form>

      {/* Custom Taxes Section */}
      {invoiceEntityId && (
        <Card className="px-8 py-6 max-w-[950px] space-y-4">
          <div className="flex justify-between items-center">
            <div>
              <h3 className="font-medium text-lg">Custom Taxes</h3>
              <p className="text-sm text-muted-foreground mt-1">
                Define custom tax rules for specific countries and regions.
              </p>
            </div>
            <Button
              size="sm"
              onClick={() => {
                setEditingCustomTax(null)
                customTaxMethods.reset({
                  name: '',
                  taxCode: '',
                  rules: [{ country: '', region: '', rate: '' }],
                })
                setCustomTaxDialogOpen(true)
              }}
            >
              <PlusIcon className="h-4 w-4 mr-2" />
              Add Custom Tax
            </Button>
          </div>

          {listCustomTaxesQuery.isLoading ? (
            <Loading />
          ) : listCustomTaxesQuery.data?.customTaxes &&
            listCustomTaxesQuery.data.customTaxes.length > 0 ? (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Tax Code</TableHead>
                  <TableHead>Rules</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {listCustomTaxesQuery.data.customTaxes.map(tax => (
                  <TableRow key={tax.id}>
                    <TableCell className="font-medium">{tax.name}</TableCell>
                    <TableCell>{tax.taxCode}</TableCell>
                    <TableCell>
                      <div className="space-y-1">
                        {tax.rules.map((rule, idx) => (
                          <div key={idx} className="text-sm">
                            {rule.country || 'All countries'}
                            {rule.region && ` - ${rule.region}`}: {rule.rate}%
                          </div>
                        ))}
                      </div>
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-2">
                        <Button
                          size="icon"
                          variant="ghost"
                          onClick={() => handleEditCustomTax(tax)}
                        >
                          <EditIcon className="h-4 w-4" />
                        </Button>
                        <Button
                          size="icon"
                          variant="ghost"
                          onClick={() => handleDeleteCustomTax(tax.id)}
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
            <div className="text-center py-8 text-muted-foreground">
              No custom taxes defined yet.
            </div>
          )}
        </Card>
      )}
      <CreateInvoicingEntityDialog
        open={createDialogOpen}
        setOpen={setCreateDialogOpen}
        setInvoicingEntity={setInvoiceEntityId}
      />

      {/* Custom Tax Dialog */}
      <Dialog open={customTaxDialogOpen} onOpenChange={setCustomTaxDialogOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{editingCustomTax ? 'Edit Custom Tax' : 'Create Custom Tax'}</DialogTitle>
            <DialogDescription>
              Define tax rules that apply to specific countries and regions.
            </DialogDescription>
          </DialogHeader>

          <Form {...customTaxMethods}>
            <form onSubmit={customTaxMethods.handleSubmit(onSubmitCustomTax)} className="space-y-4">
              <InputFormField
                name="name"
                label="Tax Name"
                placeholder="e.g., VAT, Sales Tax"
                control={customTaxMethods.control}
              />

              <InputFormField
                name="taxCode"
                label="Tax Code"
                placeholder="e.g., VAT_EU, SALES_US"
                control={customTaxMethods.control}
              />

              <div className="space-y-2">
                <label className="text-sm font-medium">Tax Rules</label>
                {customTaxMethods.watch('rules').map((_, index) => (
                  <div key={index} className="flex gap-2">
                    <InputFormField
                      name={`rules.${index}.country`}
                      placeholder="Country (optional)"
                      control={customTaxMethods.control}
                      containerClassName="flex-1"
                    />
                    <InputFormField
                      name={`rules.${index}.region`}
                      placeholder="Region (optional)"
                      control={customTaxMethods.control}
                      containerClassName="flex-1"
                    />
                    <InputFormField
                      name={`rules.${index}.rate`}
                      placeholder="Rate (%)"
                      control={customTaxMethods.control}
                      containerClassName="w-24"
                    />
                    {customTaxMethods.watch('rules').length > 1 && (
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        onClick={() => handleRemoveTaxRule(index)}
                      >
                        <Trash2Icon className="h-4 w-4" />
                      </Button>
                    )}
                  </div>
                ))}
                <Button type="button" size="sm" variant="outline" onClick={handleAddTaxRule}>
                  <PlusIcon className="h-4 w-4 mr-2" />
                  Add Rule
                </Button>
              </div>

              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setCustomTaxDialogOpen(false)
                    setEditingCustomTax(null)
                    customTaxMethods.reset()
                  }}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  disabled={
                    createCustomTaxMut.isPending ||
                    updateCustomTaxMut.isPending ||
                    !customTaxMethods.formState.isValid
                  }
                >
                  {editingCustomTax ? 'Update' : 'Create'}
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>
    </div>
  )
}
