import {
  Flex,
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
import { ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { useFormContext } from 'react-hook-form'

import { CreateCustomerSchema } from '@/lib/schemas/customers'

export const CustomersInvoice = () => {
  const { control } = useFormContext<CreateCustomerSchema>()

  const [visible, setVisible] = useState(false)

  return (
    <Flex direction="column" className="gap-2">
      <Flex
        align="center"
        className="gap-2 cursor-pointer group"
        onClick={() => setVisible(!visible)}
      >
        <h2 className="font-medium">Invoicing</h2>
        <ChevronRight
          size={14}
          className={`text-muted-foreground transition-transform duration-200 ease-in-out ${
            visible ? 'rotate-90' : ''
          }`}
        />
      </Flex>
      {visible && (
        <>
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
            name="customTaxRate"
            label="Custom tax rate (%)"
            control={control}
            type="number"
            placeholder="0"
            autoComplete="off"
          />
        </>
      )}
    </Flex>
  )
}
