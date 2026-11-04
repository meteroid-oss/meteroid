import {
  Button,
  CheckboxFormField,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  InputFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
} from '@md/ui'
import { Minus, Plus } from 'lucide-react'
import { useState } from 'react'
import { Control, useWatch } from 'react-hook-form'

import { CountrySelect } from '@/components/CountrySelect'
import { InvoiceLanguageSelect } from '@/components/InvoiceLanguageSelect'
import { CustomPropertiesFields } from '@/features/customproperties/CustomPropertiesFields'
import { CustomerFormSchema } from '@/lib/schemas/customers'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'

import { CustomerTaxFields } from './CustomerTaxFields'

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

  const customerType = useWatch({ control, name: 'customerType' })
  const isIndividual = customerType === 'INDIVIDUAL'

  return (
    <>
      {/* Customer Details Section */}
      <div className="space-y-4">
        <h3 className="font-semibold">Customer details</h3>
        <FormField
          control={control}
          name="customerType"
          render={({ field }) => (
            <FormItem className="space-y-1">
              <FormLabel>Type</FormLabel>
              <FormControl>
                <RadioGroup
                  value={field.value}
                  onValueChange={field.onChange}
                  className="flex items-center gap-4 h-8"
                >
                  <div className="flex items-center space-x-2">
                    <RadioGroupItem value="COMPANY" id="customer-type-company" />
                    <Label htmlFor="customer-type-company" className="font-normal cursor-pointer">
                      Company
                    </Label>
                  </div>
                  <div className="flex items-center space-x-2">
                    <RadioGroupItem value="INDIVIDUAL" id="customer-type-individual" />
                    <Label
                      htmlFor="customer-type-individual"
                      className="font-normal cursor-pointer"
                    >
                      Individual
                    </Label>
                  </div>
                </RadioGroup>
              </FormControl>
            </FormItem>
          )}
        />
        {!isIndividual && (
          <InputFormField
            control={control}
            required
            label="Name"
            name="name"
            layout="horizontal"
            placeholder="ACME Inc"
          />
        )}
        {isIndividual && (
          <>
            <InputFormField
              control={control}
              required
              label="First name"
              name="firstName"
              layout="horizontal"
            />
            <InputFormField
              control={control}
              required
              label="Last name"
              name="lastName"
              layout="horizontal"
            />
          </>
        )}
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
          placeholder={isIndividual ? 'jane@example.com' : 'account@company.com'}
        />
        <InputFormField
          control={control}
          label="Invoicing email"
          name="invoicingEmail"
          layout="horizontal"
          type="email"
          placeholder={isIndividual ? 'jane@example.com' : 'billing@company.com'}
        />
        <InputFormField
          control={control}
          label="Phone"
          name="phone"
          layout="horizontal"
          type="tel"
        />
        <InvoiceLanguageSelect
          control={control}
          name="preferredLanguage"
          label="Preferred language"
          layout="horizontal"
        />
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
          required={isIndividual}
          name="billingAddress.country"
          className="col-span-8 bg-input text-muted-foreground"
          layout="horizontal"
        />
      </div>

      <section className="space-y-4">
        {!isIndividual && <h3 className="font-semibold">Tax information</h3>}
        <CustomerTaxFields key={customerType} control={control} />
      </section>

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

      <CustomPropertiesFields
        control={control}
        entityType={CustomPropertyEntityType.CUSTOMER}
        titleClassName="font-semibold text-base"
        layout="horizontal"
      />
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
