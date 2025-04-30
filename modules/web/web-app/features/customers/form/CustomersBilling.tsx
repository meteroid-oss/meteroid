import { CreateCustomerSchema } from '@/lib/schemas/customers'
import { spaces } from '@md/foundation'
import {
  Checkbox,
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

export const CustomersBilling = () => {
  const { control } = useFormContext<CreateCustomerSchema>()

  return (
    <Flex direction="column" gap={spaces.space4}>
      <h2 className="font-medium">Billing Information</h2>
      <InputFormField
        name="legalName"
        label="Legal Name"
        control={control}
        type="text"
        autoComplete="off"
      />

      <FormField
        control={control}
        name="country"
        render={({ field }) => (
          <FormItem className="mb-2">
            <FormLabel>Billing details</FormLabel>
            <Select onValueChange={field.onChange} defaultValue={field.value}>
              <FormControl>
                <SelectTrigger>
                  <SelectValue placeholder="Choose a country..." />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="US">United States</SelectItem>
                <SelectItem value="UK">United Kingdom</SelectItem>
                <SelectItem value="FR">France</SelectItem>
                <SelectItem value="DE">Germany</SelectItem>
              </SelectContent>
            </Select>
            <FormMessage />
          </FormItem>
        )}
      />
      <Flex direction="column" gap={spaces.space3}>
        <InputFormField
          name="adress"
          control={control}
          type="text"
          autoComplete="off"
          placeholder="Address line 1, Example street 42"
        />
        <InputFormField
          name="adressType"
          control={control}
          type="text"
          autoComplete="off"
          placeholder="Address line 2, Apartment, suite, unit, floor etc..."
        />
        <InputFormField
          name="postalCode"
          control={control}
          type="text"
          autoComplete="off"
          placeholder="Postal code (1234 AB)"
        />
        <InputFormField
          name="city"
          control={control}
          type="text"
          autoComplete="off"
          placeholder="City"
        />
      </Flex>
      <InputFormField
        name="taxId"
        label="Tax ID"
        control={control}
        type="text"
        placeholder="NL12391234585"
        autoComplete="off"
      />

      <FormField
        control={control}
        name="shipping"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Shipping details</FormLabel>
            <FormControl>
              <Flex align="center" gap={spaces.space3}>
                <Checkbox checked={field.value} onCheckedChange={field.onChange} />
                <div className="text-xs">Same as billing details</div>
              </Flex>
            </FormControl>
          </FormItem>
        )}
      />
    </Flex>
  )
}
