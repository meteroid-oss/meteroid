import {
  Button,
  CheckboxFormField,
  Flex,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  InputFormField,
} from '@md/ui'
import { Minus, Plus, X } from 'lucide-react'
import { useState } from 'react'
import { Control, useFieldArray, useWatch } from 'react-hook-form'

import { CountrySelect } from '@/components/CountrySelect'
import { CustomerFormSchema } from '@/lib/schemas/customers'

interface CustomerFormFieldsProps<T extends CustomerFormSchema> {
  control: Control<T>
  initialShowShippingAddress?: boolean
}

export const CustomerFormFields = <T extends CustomerFormSchema>({
  control: _control,
  initialShowShippingAddress = false,
}: CustomerFormFieldsProps<T>) => {
  const [showShippingAddress, setShowShippingAddress] = useState(initialShowShippingAddress)

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const control = _control as any as Control<CustomerFormSchema>

  const { fields, append, remove } = useFieldArray({
    control,
    name: 'customTaxes',
  })

  const customTaxes = useWatch({ control, name: 'customTaxes' })
  const hasCustomTaxes = customTaxes && customTaxes.length > 0

  return (
    <>
      {/* Customer Details Section */}
      <div className="space-y-4">
        <h3 className="font-semibold">Customer details</h3>
        <InputFormField
          control={control}
          required
          label="Name"
          name="name"
          layout="horizontal"
          placeholder="ACME Inc"
        />
        <InputFormField
          control={control}
          label="Alias (external ID)"
          name="alias"
          layout="horizontal"
          placeholder="customer-r23kr"
        />
        <InputFormField
          control={control}
          label="Email"
          name="email"
          layout="horizontal"
          type="email"
          placeholder="account@company.com"
        />
        <InputFormField
          control={control}
          label="Invoicing email"
          name="invoicingEmail"
          layout="horizontal"
          type="email"
          placeholder="billing@company.com"
        />
        <InputFormField
          control={control}
          label="Phone"
          name="phone"
          layout="horizontal"
          type="tel"
        />
      </div>

      {/* Tax Information Section */}
      <div className="space-y-4">
        <h3 className="font-semibold">Tax information</h3>
        <InputFormField control={control} label="VAT number" name="vatNumber" layout="horizontal" />

        <div className="space-y-2">
          <Flex align="center" justify="between">
            <FormLabel>Custom Taxes</FormLabel>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => append({ taxCode: '', name: '', rate: 0 } as never)}
            >
              <Plus size={14} className="mr-1" />
              Add Tax
            </Button>
          </Flex>
          {fields.length > 1 && (
            <span className="text-xs text-muted-foreground">
              All taxes will be applied cumulatively to this customer&apos;s invoices
            </span>
          )}
          {fields.map((field, index) => (
            <Flex key={field.id} className="gap-2 items-start px-1">
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
        </div>

        {!hasCustomTaxes && (
          <CheckboxFormField control={control} label="Tax exempt" name="isTaxExempt" />
        )}
      </div>

      {/* Billing Address Section */}
      <div className="space-y-4">
        <h3 className="font-semibold">Billing address</h3>
        <InputFormField
          control={control}
          label="Address line 1"
          name="billingAddress.line1"
          layout="horizontal"
        />
        <InputFormField
          control={control}
          label="Address line 2"
          name="billingAddress.line2"
          layout="horizontal"
        />
        <InputFormField
          control={control}
          label="City"
          name="billingAddress.city"
          layout="horizontal"
        />
        <InputFormField
          control={control}
          label="State/Province"
          name="billingAddress.state"
          layout="horizontal"
        />
        <InputFormField
          control={control}
          label="Postal code"
          name="billingAddress.zipCode"
          layout="horizontal"
        />
        <CountrySelect
          control={control}
          label="Country"
          name="billingAddress.country"
          className="col-span-8 bg-input text-muted-foreground"
          layout="horizontal"
        />
      </div>

      {/* Shipping Address Section */}
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <h3 className="font-semibold">Shipping address</h3>
          {!showShippingAddress && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => setShowShippingAddress(true)}
              className="text-muted-foreground"
            >
              <Plus size={16} className="mr-1" />
              Add shipping address
            </Button>
          )}
        </div>

        {showShippingAddress && (
          <ShippingAddressFields control={control} onRemove={() => setShowShippingAddress(false)} />
        )}
      </div>
    </>
  )
}

interface ShippingAddressFieldsProps {
  control: Control<CustomerFormSchema>
  onRemove: () => void
}

const ShippingAddressFields = ({ control, onRemove }: ShippingAddressFieldsProps) => {
  const sameAsBilling = useWatch({ control, name: 'shippingAddress.sameAsBilling' })

  return (
    <>
      <CheckboxFormField
        control={control}
        label="Same as billing address"
        name="shippingAddress.sameAsBilling"
      />

      {!sameAsBilling && (
        <>
          <InputFormField
            control={control}
            label="Address line 1"
            name="shippingAddress.address.line1"
            layout="horizontal"
          />
          <InputFormField
            control={control}
            label="Address line 2"
            name="shippingAddress.address.line2"
            layout="horizontal"
          />
          <InputFormField
            control={control}
            label="City"
            name="shippingAddress.address.city"
            layout="horizontal"
          />
          <InputFormField
            control={control}
            label="State/Province"
            name="shippingAddress.address.state"
            layout="horizontal"
          />
          <InputFormField
            control={control}
            label="Postal code"
            name="shippingAddress.address.zipCode"
            layout="horizontal"
          />
          <CountrySelect
            control={control}
            label="Country"
            name="shippingAddress.address.country"
            className="col-span-8 bg-input text-muted-foreground"
            layout="horizontal"
          />
        </>
      )}

      <Button
        type="button"
        variant="ghost"
        size="sm"
        onClick={onRemove}
        className="text-muted-foreground"
      >
        <Minus size={16} className="mr-1" />
        Remove shipping address
      </Button>
    </>
  )
}
