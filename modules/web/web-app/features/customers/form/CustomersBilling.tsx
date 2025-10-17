import {
  Button,
  Checkbox,
  Flex,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  InputFormField,
} from '@md/ui'
import { ChevronRight, Plus, X } from 'lucide-react'
import { useState } from 'react'
import { useFieldArray, useFormContext } from 'react-hook-form'

import { CountrySelect } from '@/components/CountrySelect'
import { CreateCustomerSchema } from '@/lib/schemas/customers'

export const CustomersBilling = () => {
  const { control } = useFormContext<CreateCustomerSchema>()
  const { fields, append, remove } = useFieldArray({
    control,
    name: 'customTaxes',
  })

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
          <CountrySelect name="country" control={control} label="Billing details" />
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

          <Flex direction="column" className="gap-2">
            <Flex align="center" justify="between">
              <FormLabel>Custom Taxes</FormLabel>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => append({ taxCode: '', name: '', rate: 0 })}
              >
                <Plus size={14} className="mr-1" />
                Add Tax
              </Button>
            </Flex>
            {fields.map((field, index) => (
              <Flex key={field.id} className="gap-2 items-start">
                <FormField
                  control={control}
                  name={`customTaxes.${index}.taxCode`}
                  render={({ field }) => (
                    <FormItem className="flex-1">
                      {index === 0 && <FormLabel>Code</FormLabel>}
                      <FormControl>
                        <Input {...field} placeholder="GST" />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={control}
                  name={`customTaxes.${index}.name`}
                  render={({ field }) => (
                    <FormItem className="flex-[2]">
                      {index === 0 && <FormLabel>Name</FormLabel>}
                      <FormControl>
                        <Input {...field} placeholder="Goods and Services Tax" />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={control}
                  name={`customTaxes.${index}.rate`}
                  render={({ field }) => (
                    <FormItem className="flex-1">
                      {index === 0 && <FormLabel>Rate (%)</FormLabel>}
                      <FormControl>
                        <Input
                          {...field}
                          type="number"
                          step="0.01"
                          placeholder="5.0"
                          onChange={e => field.onChange(parseFloat(e.target.value))}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  onClick={() => remove(index)}
                  className={index === 0 ? 'mt-8' : ''}
                >
                  <X size={16} />
                </Button>
              </Flex>
            ))}
          </Flex>

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
