import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Form, Modal } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ComponentProps, useEffect, useState } from 'react'
import { toast } from 'sonner'

import {
  invoiceLanguageFromLocales,
  invoiceLanguageToLocales,
} from '@/components/InvoiceLanguageSelect'
import { OwnerField, useSaveOwner } from '@/features/customers/OwnerPicker'
import { CustomerFormFields } from '@/features/customers/form/CustomerFormFields'
import {
  parseCustomProperties,
  serializeCustomProperties,
} from '@/features/customproperties/CustomPropertiesFields'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useZodForm } from '@/hooks/useZodForm'
import {
  customerName,
  editCustomerSchema,
  EditCustomerSchema,
  taxStatusFromProto,
  taxStatusToProto,
} from '@/lib/schemas/customers'
import { percentToRate, rateToPercent } from '@/lib/utils/numbers'
import {
  getCustomerById,
  listCustomers,
  updateCustomer,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { Customer, CustomerType } from '@/rpc/api/customers/v1/models_pb'

type Props = Pick<ComponentProps<typeof Modal>, 'visible' | 'onCancel'> & {
  customer: Customer
}

export const EditCustomerModal = ({ customer, visible, onCancel }: Props) => {
  const queryClient = useQueryClient()
  const isExpress = useIsExpressOrganization()
  const saveOwner = useSaveOwner()
  const savedOwner = customer.owner?.userId
  const [owner, setOwner] = useState(savedOwner)

  const updateCustomerMutation = useMutation(updateCustomer, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getCustomerById,
          input: { id: customer.id },
          cardinality: 'finite',
        }),
      })

      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listCustomers.parent,
          cardinality: undefined,
        }),
      })
    },
  })

  const getDefaultValues = (): EditCustomerSchema => ({
    name: customer.name,
    customerType: customer.customerType === CustomerType.INDIVIDUAL ? 'INDIVIDUAL' : 'COMPANY',
    firstName: customer.firstName,
    lastName: customer.lastName,
    alias: customer.alias,
    email: customer.billingEmail,
    invoicingEmail: customer.invoicingEmails[0],
    phone: customer.phone,
    vatNumber: customer.vatNumber,
    legalNumber: customer.legalNumber,
    buyerReference: customer.buyerReference,
    customTaxes:
      customer.customTaxes?.map(tax => ({
        taxCode: tax.taxCode,
        name: tax.name,
        rate: rateToPercent(tax.rate),
      })) ?? [],
    taxStatus: taxStatusFromProto(customer.taxStatus),
    exemptionReason: customer.exemptionReason,
    billingAddress: customer.billingAddress,
    shippingAddress: customer.shippingAddress,
    preferredLanguage: invoiceLanguageFromLocales(customer.preferredLocales),
    customProperties: parseCustomProperties(customer.customProperties),
  })

  const methods = useZodForm({
    mode: 'onChange',
    schema: editCustomerSchema,
    defaultValues: getDefaultValues(),
  })

  useEffect(() => {
    if (visible) {
      methods.reset(getDefaultValues())
      setOwner(savedOwner)
    }
  }, [visible, customer])

  const onSubmit = async (data: EditCustomerSchema) => {
    await updateCustomerMutation.mutateAsync({
      customer: {
        id: customer.id,
        name: customerName(data),
        customerType:
          data.customerType === 'INDIVIDUAL' ? CustomerType.INDIVIDUAL : CustomerType.COMPANY,
        firstName: data.customerType === 'INDIVIDUAL' ? data.firstName || undefined : undefined,
        lastName: data.customerType === 'INDIVIDUAL' ? data.lastName || undefined : undefined,
        alias: data.alias,
        billingEmail: data.email,
        invoicingEmails: data.invoicingEmail ? { emails: [data.invoicingEmail] } : undefined,
        phone: data.phone,
        vatNumber: data.vatNumber,
        legalNumber: data.legalNumber ?? '',
        buyerReference: data.buyerReference ?? '',
        customTaxes: {
          taxes: (data.customTaxes || []).map(tax => ({
            taxCode: tax.taxCode,
            name: tax.name,
            rate: percentToRate(tax.rate),
          })),
        },
        taxStatus: taxStatusToProto(data.taxStatus),
        exemptionReason: data.exemptionReason ?? '',
        billingAddress: data.billingAddress,
        shippingAddress: data.shippingAddress,
        preferredLocales: invoiceLanguageToLocales(data.preferredLanguage),
        customProperties: serializeCustomProperties(data.customProperties),
      },
    })
    if (!isExpress) await saveOwner(customer.id, savedOwner, owner)
    toast.success('Customer updated')
    onCancel?.()
  }

  const hasShippingAddress = Boolean(customer.shippingAddress?.address)

  return (
    <Modal
      header={<>Edit customer</>}
      visible={visible}
      onCancel={onCancel}
      onConfirm={() => methods.handleSubmit(onSubmit)()}
      size="large"
    >
      <Modal.Content>
        <Form {...methods}>
          <form className="max-h-[70vh] overflow-y-auto pr-2">
            <div className="py-4 w-full space-y-6">
              {!isExpress && <OwnerField value={owner} onChange={setOwner} />}
              <CustomerFormFields
                control={methods.control}
                initialShowShippingAddress={hasShippingAddress}
              />
            </div>
          </form>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
