import { spaces } from '@md/foundation'
import {
  Badge,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  InputFormField,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { useEffect } from 'react'
import { useFormContext } from 'react-hook-form'

import { Combobox } from '@/components/Combobox'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useQuery } from '@/lib/connectrpc'
import { CreateCustomerSchema } from '@/lib/schemas/customers'
import { listInvoicingEntities } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

export const CustomersGeneral = ({ activeCurrencies }: {activeCurrencies: string[]}) => {
  const { control, setValue } = useFormContext<CreateCustomerSchema>()
  const listInvoicingEntitiesQuery = useQuery(listInvoicingEntities)

  useEffect(() => {
    if (listInvoicingEntitiesQuery.data?.entities) {
      const defaultEntity = listInvoicingEntitiesQuery.data.entities.find(entity => entity.isDefault)
      if (defaultEntity) {
        setValue('invoicingEntity', defaultEntity.id)
      }
    }
  }, [listInvoicingEntitiesQuery.data?.entities, setValue])

  return (
    <Flex direction="column" gap={spaces.space4}>
      <div className="font-medium">General Information</div>
      <InputFormField
        name="companyName"
        label="Name"
        control={control}
        type="text"
        placeholder="ACME Inc"
        autoComplete="off"
      />
      <InputFormField
        name="alias"
        label="Alias (external ID)"
        control={control}
        type="text"
        placeholder="customer-r23kr"
        autoComplete="off"
      />
      <InputFormField
        name="primaryEmail"
        label="Email address"
        control={control}
        type="text"
        placeholder="account@company.com"
        autoComplete="off"
      />

      <FormField
        control={control}
        name="invoicingEntity"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Invoicing entity</FormLabel>
            <Combobox
              placeholder="Select invoicing entity..."
              value={field.value}
              onChange={field.onChange}
              options={
                listInvoicingEntitiesQuery.data?.entities?.map(entity => ({
                  label: (
                    <div className="flex flex-row w-full">
                      <div className="pr-2">{getCountryFlagEmoji(entity.country)}</div>
                      <div>{entity.legalName}</div>
                      <div className="flex-grow"/>
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
            />
            <FormMessage />
          </FormItem>
        )}
      />
      <FormField
        control={control}
        name="currency"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Currency</FormLabel>
            <Select onValueChange={field.onChange} defaultValue={field.value}>
              <FormControl>
                <SelectTrigger>
                  <SelectValue placeholder="Select a currency" />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                {
                  activeCurrencies.map( (a, i) =>  <SelectItem value={a} key={`item`+i}>{a}</SelectItem> )
                }
              </SelectContent>
            </Select>
            <FormMessage />
          </FormItem>
        )}
      />
    </Flex>
  )
}
