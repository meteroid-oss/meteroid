import {
  Button,
  Form,
  InputFormField,
  SelectFormField,
  SelectItem,
  TextareaFormField,
} from '@md/ui'
import { customAlphabet } from 'nanoid'
import { FC, useEffect } from 'react'
import { z } from 'zod'

import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { createPlanSchema } from '@/lib/schemas/plans'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'

export { createPlanSchema } from '@/lib/schemas/plans'

const nanoid = customAlphabet('1234567890abcdef', 5)

export const generateFromName = (name: string) => {
  const convert = (separator: string, a: string) => {
    return a
      .split(/_|-|(![A-Z])(?=[A-Z][a-z])|(?![^A-Z_-])(?=[A-Z])|(?![A-Za-z])(?=[^A-Za-z])/g)
      ?.filter(a => a)
      ?.map(a => a.toLowerCase())
      .join(separator);
  }
  const normalized = name.replaceAll(/[^A-Za-z\d_\- ]/g, '')
  const converted = normalized.includes(' ')
    ? normalized
        .split(' ')
        .map(a => convert('_', a))
        .join('-')
    : convert('-', normalized)
  return !converted || converted.length === 0 ? nanoid() : converted
}

interface Props {
  onCancel: () => void
  onNext: (values: z.infer<typeof createPlanSchema>) => void
  defaultValues?: Partial<z.infer<typeof createPlanSchema>>
  submitLabel?: string
  isSubmitting?: boolean
}
export const DetailsForm: FC<Props> = ({ onCancel, onNext, defaultValues, submitLabel = 'Next →', isSubmitting }) => {
  const familiesQuery = useQuery(listProductFamilies)
  const families = (familiesQuery.data?.productFamilies ?? []).sort((a, b) =>
    a.id > b.id ? 1 : -1
  )
  const methods = useZodForm({
    schema: createPlanSchema,
    defaultValues,
    mode: 'onChange',
  })

  const onSubmit = (data: z.infer<typeof createPlanSchema>) => {
    onNext(data)
  }

  useEffect(() => {
    if (!defaultValues?.productFamilyLocalId) {
      methods.setValue('productFamilyLocalId', families[0]?.localId)
    }
  }, [families])

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <section className="space-y-4">
          <div className="space-y-6 py-2">
            <InputFormField
              name="planName"
              label="Name"
              layout="horizontal"
              control={methods.control}
              type="text"
              placeholder="Plan name"
            />

            {families.length > 1 ? (
              <SelectFormField
                name="productFamilyLocalId"
                label="Product line"
                layout="horizontal"
                placeholder="Select a product line"
                className="max-w-[320px]  "
                empty={families.length === 0}
                control={methods.control}
              >
                {families.map(f => (
                  <SelectItem value={f.localId} key={f.localId}>
                    {f.name}
                  </SelectItem>
                ))}
              </SelectFormField>
            ) : (
              <InputFormField
                hidden
                className="hidden"
                value={families[0]?.localId}
                control={methods.control}
                name="productFamilyLocalId"
              />
            )}

            {/* TODO */}
            <div className="hidden">
              <div className="w-full border-b "></div>

              <TextareaFormField
                name="description"
                label="Description"
                control={methods.control}
                placeholder="This plan gives access to ..."
                layout="horizontal"
              />
            </div>
          </div>

          <div className="flex justify-end w-full items-center space-x-3">
            <Button variant="secondary" onClick={onCancel}>
              Cancel
            </Button>

            <Button variant="primary" type="submit" disabled={!methods.formState.isValid || isSubmitting}>
              {submitLabel}
            </Button>
          </div>
        </section>
      </form>
    </Form>
  )
}
