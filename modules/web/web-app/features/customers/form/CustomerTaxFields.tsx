import { Button, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { Plus, X } from 'lucide-react'
import { ReactNode, useState } from 'react'
import { Control, useFieldArray, useWatch } from 'react-hook-form'

import { CustomerFormSchema } from '@/lib/schemas/customers'

export const CustomerTaxFields = ({ control }: { control: Control<CustomerFormSchema> }) => {
  const { fields, append, remove } = useFieldArray({ control, name: 'customTaxes' })
  const taxStatus = useWatch({ control, name: 'taxStatus' })
  const customerType = useWatch({ control, name: 'customerType' })
  const vatNumber = useWatch({ control, name: 'vatNumber' })
  const legalNumber = useWatch({ control, name: 'legalNumber' })
  const buyerReference = useWatch({ control, name: 'buyerReference' })
  const isIndividual = customerType === 'INDIVIDUAL'
  const exemptionReason = useWatch({ control, name: 'exemptionReason' })
  const configured = Boolean(
    fields.length ||
    (taxStatus && taxStatus !== 'TAXABLE') ||
    exemptionReason ||
    (isIndividual && (vatNumber || legalNumber || buyerReference))
  )

  const identifiers = (
    <>
      {(!isIndividual || vatNumber) && (
        <InputFormField control={control} label="VAT number" name="vatNumber" layout="horizontal" />
      )}
      {(!isIndividual || legalNumber) && (
        <InputFormField
          control={control}
          label="Registration number"
          name="legalNumber"
          layout="horizontal"
        />
      )}
      {(!isIndividual || buyerReference) && (
        <InputFormField
          control={control}
          label="Buyer reference"
          name="buyerReference"
          layout="horizontal"
          placeholder="Leitweg-ID, for XRechnung"
        />
      )}
    </>
  )

  return (
    <div className="space-y-4">
      {!isIndividual && identifiers}
      <AdvancedTaxSettings configured={configured}>
        {isIndividual && identifiers}
        <div className="grid gap-2 items-start md:grid-cols-12">
          <div className="md:col-span-4 pt-2 text-xs text-muted-foreground">Custom taxes</div>
          <div className="md:col-span-8 min-w-0 space-y-3">
            {fields.map((field, index) => (
              <div key={field.id} className="relative space-y-2 rounded-md border p-3">
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="absolute right-1 top-1 h-7 w-7"
                  aria-label={`Remove custom tax ${index + 1}`}
                  onClick={() => remove(index)}
                >
                  <X size={14} />
                </Button>
                <InputFormField
                  control={control}
                  name={`customTaxes.${index}.taxCode`}
                  label="Code"
                  placeholder="GST"
                />
                <InputFormField
                  control={control}
                  name={`customTaxes.${index}.name`}
                  label="Name"
                  placeholder="Goods and Services Tax"
                />
                <InputFormField
                  control={control}
                  name={`customTaxes.${index}.rate`}
                  label="Rate (%)"
                  type="number"
                  step="0.01"
                  min={0}
                  max={100}
                />
              </div>
            ))}
            {fields.length > 1 && (
              <p className="text-xs text-muted-foreground">
                All custom taxes are applied cumulatively to this customer&apos;s invoices.
              </p>
            )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => append({ taxCode: '', name: '', rate: 0 })}
            >
              <Plus size={14} className="mr-1" /> Add tax
            </Button>
          </div>
        </div>
        {fields.length === 0 && (
          <>
            <SelectFormField
              control={control}
              label="Tax override"
              name="taxStatus"
              layout="horizontal"
              className="min-w-0 w-full"
            >
              <SelectItem value="TAXABLE">Automatic (engine)</SelectItem>
              <SelectItem value="EXEMPT">Exempt</SelectItem>
              <SelectItem value="REVERSE_CHARGE">Reverse charge</SelectItem>
            </SelectFormField>
            <p className="text-xs text-muted-foreground">
              Automatic uses the tax engine. Exempt and Reverse charge override its result.
            </p>
            {(taxStatus === 'EXEMPT' || taxStatus === 'REVERSE_CHARGE') && (
              <InputFormField
                control={control}
                label="Exemption reason"
                name="exemptionReason"
                layout="horizontal"
                placeholder="Legal mention shown on invoices"
              />
            )}
          </>
        )}
      </AdvancedTaxSettings>
    </div>
  )
}

const AdvancedTaxSettings = ({
  children,
  configured,
}: {
  children: ReactNode
  configured: boolean
}) => {
  const [open, setOpen] = useState(configured)
  return (
    <details
      open={open}
      onToggle={event => setOpen(event.currentTarget.open)}
      className="space-y-4"
    >
      <summary className="cursor-pointer text-sm font-medium">
        Advanced tax settings{configured ? ' (configured)' : ''}
      </summary>
      {configured && (
        <p className="text-xs text-muted-foreground">
          These tax settings still apply to this customer. Review them when changing customer type.
        </p>
      )}
      {children}
    </details>
  )
}
