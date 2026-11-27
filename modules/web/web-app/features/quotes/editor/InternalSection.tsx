import { Input, Label, TextareaFormField } from '@md/ui'
import { EyeOff } from 'lucide-react'
import { useFormContext } from 'react-hook-form'

import { CustomPropertiesFields } from '@/features/customproperties/CustomPropertiesFields'
import { EditorSection } from '@/features/quotes/editor/EditorSection'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { QuoteV2State } from '@/features/quotes/editor/quoteV2'
import type { Dispatch, SetStateAction } from 'react'

interface InternalSectionProps {
  hubspotDealId: string
  setV2: Dispatch<SetStateAction<QuoteV2State>>
  allowHubspot: boolean
}

export const InternalSection = ({ hubspotDealId, setV2, allowHubspot }: InternalSectionProps) => {
  const { control } = useFormContext<QuoteFormValues>()

  return (
    <EditorSection
      id="quote-internal"
      title={
        <span className="flex items-center gap-1.5">
          Internal
          <span title="Not shown to the customer" className="text-muted-foreground">
            <EyeOff className="h-4 w-4" aria-hidden />
            <span className="sr-only">Not shown to the customer</span>
          </span>
        </span>
      }
    >
      {allowHubspot && (
        <div className="flex flex-col @lg:max-w-[50%]">
          <Label htmlFor="quoteHubspotDeal" className="my-2 text-xs font-normal">
            HubSpot deal ID or link
          </Label>
          <Input
            id="quoteHubspotDeal"
            value={hubspotDealId}
            onChange={e => setV2(prev => ({ ...prev, hubspotDealId: e.target.value }))}
            placeholder="https://app.hubspot.com/contacts/…/record/0-3/…"
          />
        </div>
      )}
      <TextareaFormField
        name="internal_notes"
        label="Internal notes"
        control={control}
        placeholder="Notes for your team"
        layout="vertical"
        rows={3}
      />
      <CustomPropertiesFields
        control={control}
        entityType={CustomPropertyEntityType.QUOTE}
        title={null}
      />
    </EditorSection>
  )
}
