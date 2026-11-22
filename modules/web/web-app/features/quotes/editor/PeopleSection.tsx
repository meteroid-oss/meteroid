import {
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  DateFormField,
  GenericFormField,
  Input,
  InputFormField,
  Label,
  Switch,
  Textarea,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
  cn,
} from '@md/ui'
import { ChevronRight, InfoIcon, Plus, Trash2 } from 'lucide-react'
import { useFieldArray, useFormContext } from 'react-hook-form'

import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { defaultConsentText } from '@/features/quotes/detail/signers'
import { EditorSection } from '@/features/quotes/editor/EditorSection'
import { QuoteSignatureLevel } from '@/rpc/api/quotes/v1/models_pb'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type {
  CountersignerState,
  CustomerActionsState,
  QuoteV2State,
} from '@/features/quotes/editor/quoteV2'
import type { Dispatch, SetStateAction } from 'react'

const LEVEL_HELP: Record<QuoteSignatureLevel, string> = {
  [QuoteSignatureLevel.SIMPLE]:
    'The signer types or draws a signature; their IP address and browser are recorded.',
  [QuoteSignatureLevel.VERIFIED]:
    'The signer also confirms a code emailed to them, and a sealed completion certificate is issued.',
}

const CUSTOMER_ACTIONS: { key: keyof CustomerActionsState; label: string }[] = [
  { key: 'questions', label: 'Ask questions' },
  { key: 'changeRequests', label: 'Request changes' },
  { key: 'decline', label: 'Decline' },
]

interface PeopleSectionProps {
  v2: QuoteV2State
  setV2: Dispatch<SetStateAction<QuoteV2State>>
  allowMultipleRecipients: boolean
  allowVerified: boolean
  allowCountersigners: boolean
  allowCustomerActions: boolean
  /** Adding or removing a recipient is no field change: the autosave is told directly. */
  onRecipientsChange: () => void
  /** Customer, expiry and recipients; a template has none. */
  dealFields?: boolean
}

export const PeopleSection = ({
  v2,
  setV2,
  allowMultipleRecipients,
  allowVerified,
  allowCountersigners,
  allowCustomerActions,
  onRecipientsChange,
  dealFields = true,
}: PeopleSectionProps) => {
  const { control } = useFormContext<QuoteFormValues>()
  const recipients = useFieldArray({ control, name: 'recipients' })

  const setCountersigners = (update: (list: CountersignerState[]) => CountersignerState[]) =>
    setV2(prev => ({ ...prev, countersigners: update(prev.countersigners) }))

  const updateCountersigner = (index: number, patch: Partial<CountersignerState>) =>
    setCountersigners(list => list.map((c, i) => (i === index ? { ...c, ...patch } : c)))

  return (
    <EditorSection id="quote-people" title={dealFields ? 'Customer & people' : 'Signing'}>
      <div className="flex flex-col divide-y divide-border">
        {dealFields && (
          <div className="grid grid-cols-1 gap-4 md:grid-cols-3 py-4 first:pt-0 last:pb-0">
            <GenericFormField
              control={control}
              layout="vertical"
              label="Customer"
              name="customer_id"
              render={({ field }) => (
                <CustomerSelect value={field.value} onChange={field.onChange} />
              )}
            />
            <DateFormField
              name="expires_at"
              label="Expires on"
              control={control}
              layout="vertical"
              clearable
            />
            <InputFormField
              name="quote_number"
              label="Quote number"
              control={control}
              placeholder="Q-12345"
              layout="vertical"
            />
          </div>
        )}

        {(dealFields || allowCountersigners) && (
          <div className="flex flex-col gap-3 py-4 first:pt-0 last:pb-0">
            {dealFields && (
              <>
                <h3 className="text-sm font-medium">Who signs</h3>
                <div className="flex flex-col gap-2">
                  {recipients.fields.map((field, index) => (
                    <div key={field.id} className="flex items-start gap-2">
                      <div className="grid flex-1 grid-cols-1 gap-2 sm:grid-cols-2">
                        <InputFormField
                          name={`recipients.${index}.name`}
                          control={control}
                          placeholder="Name"
                          aria-label={`Recipient ${index + 1} name`}
                        />
                        <InputFormField
                          name={`recipients.${index}.email`}
                          control={control}
                          type="email"
                          placeholder="email@company.com"
                          aria-label={`Recipient ${index + 1} email`}
                        />
                      </div>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        disabled={recipients.fields.length === 1}
                        onClick={() => {
                          recipients.remove(index)
                          onRecipientsChange()
                        }}
                        aria-label={`Remove recipient ${index + 1}`}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  ))}
                  {allowMultipleRecipients && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      hasIcon
                      className="self-start"
                      onClick={() => {
                        recipients.append({ name: '', email: '' })
                        onRecipientsChange()
                      }}
                    >
                      <Plus className="h-4 w-4" />
                      Add recipient
                    </Button>
                  )}
                </div>
              </>
            )}

            {allowCountersigners && (
              <div className="flex flex-col gap-2">
                {v2.countersigners.length > 0 && (
                  <h4 className="text-sm font-medium">Countersigners</h4>
                )}
                {v2.countersigners.map((signer, index) => (
                  <div
                    key={index}
                    className="flex flex-wrap items-center gap-2 rounded-md border border-border p-2"
                  >
                    <div className="grid min-w-0 flex-1 grid-cols-1 gap-2 sm:grid-cols-3">
                      <Input
                        value={signer.name}
                        onChange={e => updateCountersigner(index, { name: e.target.value })}
                        placeholder="Name"
                        aria-label={`Countersigner ${index + 1} name`}
                      />
                      <Input
                        type="email"
                        value={signer.email}
                        onChange={e => updateCountersigner(index, { email: e.target.value })}
                        placeholder="Email"
                        aria-label={`Countersigner ${index + 1} email`}
                      />
                      <Input
                        value={signer.title}
                        onChange={e => updateCountersigner(index, { title: e.target.value })}
                        placeholder="Title (optional)"
                        aria-label={`Countersigner ${index + 1} title`}
                      />
                    </div>
                    <div className="flex items-center gap-2">
                      <Switch
                        id={`countersignerAutomatic${index}`}
                        checked={signer.automatic}
                        onCheckedChange={checked =>
                          updateCountersigner(index, { automatic: checked })
                        }
                      />
                      <Label
                        htmlFor={`countersignerAutomatic${index}`}
                        className="whitespace-nowrap text-sm font-normal"
                      >
                        Sign automatically
                      </Label>
                    </div>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={() => setCountersigners(list => list.filter((_, i) => i !== index))}
                      aria-label={`Remove countersigner ${index + 1}`}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  hasIcon
                  className="self-start"
                  onClick={() =>
                    setCountersigners(list => [
                      ...list,
                      { name: '', email: '', title: '', automatic: false },
                    ])
                  }
                >
                  <Plus className="h-4 w-4" />
                  Add countersigner
                </Button>
              </div>
            )}
          </div>
        )}

        <div className="flex flex-col gap-3 py-4 first:pt-0 last:pb-0">
          {allowVerified && (
            <div className="flex flex-wrap items-center gap-3">
              <span className="flex items-center gap-1">
                <span id="quoteSignatureLevel" className="text-sm font-medium">
                  Signature
                </span>
                <TooltipProvider delayDuration={100}>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <InfoIcon
                        className="h-4 w-4 cursor-help text-muted-foreground"
                        aria-label="About signature levels"
                      />
                    </TooltipTrigger>
                    <TooltipContent className="max-w-xs">
                      Simple: {LEVEL_HELP[QuoteSignatureLevel.SIMPLE]}
                      <br />
                      Verified: {LEVEL_HELP[QuoteSignatureLevel.VERIFIED]}
                    </TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </span>
              <div
                role="group"
                aria-labelledby="quoteSignatureLevel"
                className="inline-flex overflow-hidden rounded-md border border-border"
              >
                {[
                  { level: QuoteSignatureLevel.SIMPLE, label: 'Simple' },
                  { level: QuoteSignatureLevel.VERIFIED, label: 'Verified' },
                ].map(({ level, label }) => (
                  <button
                    key={level}
                    type="button"
                    aria-pressed={v2.signatureLevel === level}
                    onClick={() => setV2(prev => ({ ...prev, signatureLevel: level }))}
                    className={cn(
                      'h-8 px-3 text-sm transition-colors',
                      v2.signatureLevel === level
                        ? 'bg-primary text-primary-foreground'
                        : 'bg-background text-muted-foreground hover:bg-muted hover:text-foreground'
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          )}

          <Collapsible defaultOpen={v2.consentText.trim() !== ''}>
            <CollapsibleTrigger className="group flex items-center gap-1 text-sm text-brand hover:underline">
              <ChevronRight className="h-4 w-4 transition-transform group-data-[state=open]:rotate-90" />
              Acceptance statement{v2.consentText.trim() ? ' (custom)' : ''}
            </CollapsibleTrigger>
            <CollapsibleContent className="flex flex-col gap-1.5 pt-2">
              <Label htmlFor="quoteConsentText" className="sr-only">
                Acceptance statement
              </Label>
              <Textarea
                id="quoteConsentText"
                value={v2.consentText}
                onChange={e => setV2(prev => ({ ...prev, consentText: e.target.value }))}
                placeholder={defaultConsentText(v2.attachments.length > 0)}
                rows={2}
              />
            </CollapsibleContent>
          </Collapsible>
        </div>

        {allowCustomerActions && (
          <div className="flex flex-wrap items-center gap-x-6 gap-y-3 py-4 first:pt-0 last:pb-0">
            <span className="text-sm font-medium">Customer can</span>
            {CUSTOMER_ACTIONS.map(({ key, label }) => (
              <div key={key} className="flex items-center gap-2">
                <Switch
                  id={`quoteCustomerAction-${key}`}
                  checked={v2.customerActions[key]}
                  onCheckedChange={checked =>
                    setV2(prev => ({
                      ...prev,
                      customerActions: { ...prev.customerActions, [key]: checked },
                    }))
                  }
                />
                <Label htmlFor={`quoteCustomerAction-${key}`} className="text-sm font-normal">
                  {label}
                </Label>
              </div>
            ))}
          </div>
        )}
      </div>
    </EditorSection>
  )
}
