import { Button, Card, Form, InputFormField, TextareaFormField, Label } from '@ui2/components'
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
    <>
      <Form {...methods}>
        <form
          onSubmit={methods.handleSubmit(async values => {
            alert('Not implemented')
          })}
        >
          <Card className="px-8 py-6 mb-4">
            <div className="w-full space-y-4">
              <TextareaFormField
                name="invoiceMemo"
                className="max-w-xs"
                control={methods.control}
                label="Default memo (optional)"
                layout="horizontal"
              />

              <TextareaFormField
                name="invoiceFooter"
                className="max-w-xs"
                control={methods.control}
                label="Footer"
                layout="horizontal"
              />

              <InputFormField
                label="Invoice Number Prefix"
                layout="horizontal"
                className="max-w-xs"
                type="text"
                placeholder="INV"
                control={methods.control}
                name="invoiceNumberPrefix"
              />

              <InputFormField
                label="Credit Note Number Prefix"
                layout="horizontal"
                className="max-w-xs"
                type="text"
                placeholder="CN"
                control={methods.control}
                name="creditNoteNumberPrefix"
              />

              <InputFormField
                label="Default Payment Terms"
                layout="horizontal"
                className="max-w-xs"
                type="number"
                placeholder="30"
                step={1}
                control={methods.control}
                name="defaultPaymentTerms"
              />
            </div>

            <div className="w-full flex justify-end items-center pt-6">
              <small className="text-muted-foreground">Not implemented</small>
              <Button variant="ghost" className="!rounded-r-none" size="sm">
                Cancel
              </Button>
              <Button className="!rounded-l-none" size="sm" disabled={!methods.formState.isValid}>
                Save
              </Button>
            </div>
          </Card>
        </form>
      </Form>
      <LogoForm />
    </>
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
    <Form {...methods}>
      <form
        onSubmit={methods.handleSubmit(async values => {
          alert('Not implemented')
        })}
      >
        <Card className="px-8 py-6 mb-4">
          <InputFormField
            label="Company logo"
            layout="horizontal"
            type="file"
            placeholder="Zip"
            control={methods.control}
            name="logo"
          />

          <div className="w-full flex justify-end items-center pt-6">
            <small className="text-muted-foreground">Not implemented</small>
            <Button variant="ghost" className="!rounded-r-none" size="sm">
              Cancel
            </Button>
            <Button className="!rounded-l-none" size="sm" disabled={!methods.formState.isValid}>
              Save
            </Button>
          </div>
        </Card>
      </form>
    </Form>
  )
}
