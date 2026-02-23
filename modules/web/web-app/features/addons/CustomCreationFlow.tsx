import { Button, Form, InputFormField } from '@md/ui'
import { ArrowLeftIcon, PencilIcon } from 'lucide-react'
import { createElement } from 'react'
import { z } from 'zod'

import { FeeTypePicker } from '@/features/plans/pricecomponents/FeeTypePicker'
import { ProductPricingForm } from '@/features/plans/pricecomponents/ProductPricingForm'
import { feeTypeIcon, feeTypeToHuman } from '@/features/plans/pricecomponents/utils'

import type { ComponentFeeType } from '@/features/pricing/conversions'

export const IdentitySchema = z.object({
  productName: z.string().min(1, 'Product name is required'),
  description: z.string().max(2048).optional(),
})

interface CustomCreationFlowProps {
  step: 'identity' | 'feeType' | 'form'
  name: string
  description: string
  selectedFeeType: ComponentFeeType | null
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  identityMethods: any
  currency: string
  onIdentitySubmit: (data: { productName: string; description?: string }) => void
  onFeeTypeSelect: (feeType: ComponentFeeType) => void
  onBack: (step: 'identity' | 'feeType') => void
  onSubmit: (formData: Record<string, unknown>) => void
  submitLabel?: string
}

export const CustomCreationFlow = ({
  step,
  name,
  description,
  selectedFeeType,
  identityMethods,
  currency,
  onIdentitySubmit,
  onFeeTypeSelect,
  onBack,
  onSubmit,
  submitLabel = 'Create Add-on',
}: CustomCreationFlowProps) => {
  switch (step) {
    case 'identity':
      return (
        <Form {...identityMethods}>
          <div className="space-y-4">
            <InputFormField
              name="productName"
              label="Product name"
              control={identityMethods.control}
            />
            <InputFormField
              name="description"
              label="Description (optional)"
              control={identityMethods.control}
            />
            <div className="flex justify-end pt-2">
              <Button type="button" onClick={identityMethods.handleSubmit(onIdentitySubmit)}>
                Next
              </Button>
            </div>
          </div>
        </Form>
      )
    case 'feeType':
      return (
        <div className="space-y-4">
          <button
            type="button"
            onClick={() => onBack('identity')}
            className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            <ArrowLeftIcon size={14} />
            Back
          </button>
          <FeeTypePicker onSelect={onFeeTypeSelect} />
        </div>
      )
    case 'form':
      if (!selectedFeeType) return null
      return (
        <div className="space-y-4">
          <button
            type="button"
            onClick={() => onBack('feeType')}
            className="flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            <ArrowLeftIcon size={14} />
            Back
          </button>
          <div className="rounded-lg border border-border bg-card">
            <div className="flex items-center gap-3 px-4 py-3 border-b border-border">
              <span className="text-muted-foreground">
                {createElement(feeTypeIcon(selectedFeeType), { size: 20 })}
              </span>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium truncate">{name}</span>
                  <button
                    type="button"
                    onClick={() => onBack('identity')}
                    className="text-muted-foreground hover:text-foreground transition-colors"
                  >
                    <PencilIcon size={12} />
                  </button>
                </div>
                {description && (
                  <span className="text-xs text-muted-foreground truncate block">
                    {description}
                  </span>
                )}
              </div>
              <span className="text-xs text-muted-foreground">
                {feeTypeToHuman(selectedFeeType)}
              </span>
            </div>
            <div className="p-4">
              <ProductPricingForm
                feeType={selectedFeeType}
                currency={currency}
                editableStructure
                onSubmit={onSubmit}
                submitLabel={submitLabel}
              />
            </div>
          </div>
        </div>
      )
  }
}
