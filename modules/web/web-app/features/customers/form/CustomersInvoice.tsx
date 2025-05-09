import { CreateCustomerSchema } from '@/lib/schemas/customers'
import { spaces } from '@md/foundation'
import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  InputFormField,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { useFormContext } from 'react-hook-form'

export const CustomersInvoice = () => {
  const { control } = useFormContext<CreateCustomerSchema>()

  return (
    <Flex direction="column" gap={spaces.space4}>
      <h2 className="font-medium">Invoicing</h2>

      <FormField
        control={control}
        name="paymentMethod"
        render={({ field }) => (
          <FormItem>
            <FormLabel>Payment Method</FormLabel>
            <Select onValueChange={field.onChange} defaultValue={field.value}>
              <FormControl>
                <SelectTrigger>
                  <SelectValue placeholder="Select a payment method..." />
                </SelectTrigger>
              </FormControl>
              <SelectContent>
                <SelectItem value="creditCard">Credit Card</SelectItem>
                <SelectItem value="bankTransfer">Bank Transfer</SelectItem>
                <SelectItem value="directDebit">Direct Debit</SelectItem>
                <SelectItem value="stripe">Stripe</SelectItem>
              </SelectContent>
            </Select>
            <FormMessage />
          </FormItem>
        )}
      />
      <InputFormField
        name="stripeCustomerId"
        label="Stripe ID"
        control={control}
        type="text"
        autoComplete="off"
      />

      <div className="grid grid-cols-2 gap-4">
        <FormField
          control={control}
          name="paymentTerm"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Payment Term (days)</FormLabel>
              <FormControl>
                <Input
                  type="number"
                  {...field}
                  onChange={e => field.onChange(parseInt(e.target.value) || 0)}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />

        <FormField
          control={control}
          name="gracePeriod"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Grace Period (hours)</FormLabel>
              <FormControl>
                <Input
                  type="number"
                  {...field}
                  onChange={e => field.onChange(parseInt(e.target.value) || 0)}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      </div>
      <InputFormField
        name="taxRate"
        label="Custom tax rate (%)"
        control={control}
        type="text"
        placeholder="0"
        autoComplete="off"
      />
    </Flex>
  )
}
