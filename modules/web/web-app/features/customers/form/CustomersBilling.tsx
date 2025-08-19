import {
  Checkbox,
  Flex,
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
import { ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { useFormContext } from 'react-hook-form'

import { CreateCustomerSchema } from '@/lib/schemas/customers'

export const CustomersBilling = () => {
  const { control } = useFormContext<CreateCustomerSchema>()

  const [visible, setVisible] = useState(false)

  return (
    <Flex direction="column" className="gap-2">
      <Flex
        align="center"
        className="gap-2 cursor-pointer group"
        onClick={() => setVisible(!visible)}
      >
        <h2 className="font-medium">Billing Information</h2>
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
          <Flex direction="column" className="gap-1.5">
            <InputFormField
              name="addressLine1"
              control={control}
              type="text"
              autoComplete="off"
              placeholder="Address line 1, Example street 42"
            />
            <InputFormField
              name="addressLine2"
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
            name="vatNumber"
            label="VAT Number"
            control={control}
            type="text"
            placeholder="NL123456789B01"
            autoComplete="off"
          />
          
          <InputFormField
            name="customTaxRate"
            label="Custom Tax Rate (%)"
            control={control}
            type="text"
            placeholder="21.5"
            autoComplete="off"
          />
          
          <FormField
            control={control}
            name="isTaxExempt"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Tax Exemption</FormLabel>
                <FormControl>
                  <Flex align="center" className="gap-1.5">
                    <Checkbox checked={field.value} onCheckedChange={field.onChange} />
                    <div className="text-xs">Tax exempt customer</div>
                  </Flex>
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />

          <FormField
            control={control}
            name="shipping"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Shipping details</FormLabel>
                <FormControl>
                  <Flex align="center" className="gap-1.5">
                    <Checkbox checked={field.value} onCheckedChange={field.onChange} />
                    <div className="text-xs">Same as billing details</div>
                  </Flex>
                </FormControl>
              </FormItem>
            )}
          />
        </>
      )}
    </Flex>
  )
}
