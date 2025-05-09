import { CreateCustomerSchema } from '@/lib/schemas/customers'
import { spaces } from '@md/foundation'
import {
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
import { useFormContext } from 'react-hook-form'

export const CustomersGeneral = () => {
  const { control } = useFormContext<CreateCustomerSchema>()

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
            <Select onValueChange={field.onChange} defaultValue={field.value}>
              <FormControl>
                <SelectTrigger>
                  <SelectValue placeholder="Select invoicing entity..." />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="entity1">Acme inc</SelectItem>
                <SelectItem value="entity2">Example entity 2</SelectItem>
              </SelectContent>
            </Select>
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
                <SelectItem value="usd">USD - US Dollar</SelectItem>
                <SelectItem value="eur">EUR - Euro</SelectItem>
                <SelectItem value="my">My currency</SelectItem>
              </SelectContent>
            </Select>
            <FormMessage />
          </FormItem>
        )}
      />
    </Flex>
  )
}
