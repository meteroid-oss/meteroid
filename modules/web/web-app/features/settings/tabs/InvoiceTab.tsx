import { ButtonAlt, FormItem, Input, Textarea } from '@ui/components'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'

const invoicingSchema = z.object({
  invoiceMemo: z.string().optional(),
  invoiceFooter: z.string().optional(),
  invoiceNumberPrefix: z.string().min(1),
  creditNoteNumberPrefix: z.string().min(1),
  defaultPaymentTerms: z.number().nonnegative().int(),
})

export const InvoiceTab = () => {
  const methods = useZodForm({
    schema: invoicingSchema,
  })

  return (
    <div>
      <div className="max-w-3xl space-y-2 px-10 py-6 mt-6 border border-slate-500 rounded-lg bg-white-100 dark:bg-white-200">
        <div className="w-full space-y-4">
          <FormItem label="Default memo (optional)" layout="horizontal">
            <Textarea className="max-w-xs" {...methods.register('invoiceMemo')} />
          </FormItem>
          <FormItem label="Footer" layout="horizontal">
            <Textarea
              className="max-w-xs w-xs"
              placeholder="Thank you for your business!"
              {...methods.register('invoiceFooter')}
            />
          </FormItem>
          <FormItem label="Invoice Number Prefix" layout="horizontal">
            <Input
              className="max-w-xs"
              type="text"
              placeholder="INV"
              {...methods.register('invoiceNumberPrefix')}
            />
          </FormItem>
          <FormItem label="Credit Note Number Prefix" layout="horizontal">
            <Input
              className="max-w-xs"
              type="text"
              placeholder="CN"
              {...methods.register('creditNoteNumberPrefix')}
            />
          </FormItem>
          <FormItem label="Default Payment Terms" layout="horizontal">
            <Input
              className="max-w-xs"
              type="number"
              placeholder="30"
              step={1}
              {...methods.register('defaultPaymentTerms', {
                valueAsNumber: true,
              })}
              {...methods.withError('defaultPaymentTerms')}
            />
          </FormItem>
        </div>

        <div className="w-full flex justify-end pt-6">
          <small>Not implemented</small>
          <ButtonAlt type="default" className="!rounded-r-none">
            Cancel
          </ButtonAlt>
          <ButtonAlt className="!rounded-l-none" disabled={!methods.formState.isValid}>
            Save
          </ButtonAlt>
        </div>
      </div>
      <LogoForm />
    </div>
  )
}

const logoUploadSchema = z.object({
  logo: z.string().min(1),
})

export const LogoForm = () => {
  const methods = useZodForm({
    schema: logoUploadSchema,
  })

  return (
    <div className="max-w-3xl space-y-2 px-10 py-6 mt-6 border border-slate-500 rounded-lg bg-white-100 dark:bg-white-200">
      <FormItem name="logo" label="Company logo" {...methods.withError('logo')} layout="horizontal">
        <Input type="file" placeholder="Zip" {...methods.register('logo')} />
      </FormItem>
      <div className="w-full flex justify-end pt-6">
        <small>Not implemented</small>
        <ButtonAlt type="default" className="!rounded-r-none">
          Cancel
        </ButtonAlt>
        <ButtonAlt className="!rounded-l-none" disabled={!methods.formState.isValid}>
          Save
        </ButtonAlt>
      </div>
    </div>
  )
}
