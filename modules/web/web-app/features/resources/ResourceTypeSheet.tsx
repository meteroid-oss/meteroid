import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  GenericFormField,
  InputFormField,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { InfoIcon, PlusIcon, XIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useFieldArray, useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import ConfirmationModal from '@/components/ConfirmationModal'
import { ValuesTagInput } from '@/components/form'
import { TypeCoverage } from '@/features/resources/TypeDetail'
import { useInvalidateResources } from '@/features/resources/hooks'
import { useZodForm } from '@/hooks/useZodForm'
import {
  createResourceType,
  updateResourceType,
} from '@/rpc/api/resources/v1/resources-ResourceTypesService_connectquery'

import type { ResourceType } from '@/rpc/api/resources/v1/models_pb'

const IDENTIFIER = /^[a-z][a-z0-9_]*$/
const IDENTIFIER_MESSAGE = 'Lowercase letters, digits and _, starting with a letter'

const schema = z.object({
  name: z.string().trim().min(1, 'Required'),
  pluralName: z.string().trim().min(1, 'Required'),
  code: z.string().trim().regex(IDENTIFIER, IDENTIFIER_MESSAGE),
  keys: z
    .array(z.string().trim().min(1))
    .min(1, 'Add at least one key')
    .refine(keys => new Set(keys).size === keys.length, 'Keys must be unique'),
  attributes: z
    .array(
      z.object({
        name: z.string().trim().regex(IDENTIFIER, IDENTIFIER_MESSAGE),
        values: z.array(z.string()).min(1, 'Add at least one value'),
      })
    )
    .refine(
      attrs => new Set(attrs.map(a => a.name.trim())).size === attrs.length,
      'Attribute codes must be unique'
    ),
})

type FormData = z.infer<typeof schema>

const toForm = (t?: ResourceType): FormData => ({
  name: t?.name ?? '',
  pluralName: t?.pluralName ?? '',
  code: t?.code ?? '',
  keys: [...(t?.keys ?? [])],
  attributes: Object.entries(t?.attributes ?? {})
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, v]) => ({ name, values: [...v.values] })),
})

const slugify = (name: string) =>
  name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^[^a-z]+/, '')
    .replace(/_+$/, '')

type Props = {
  type?: ResourceType
  onClose: (created?: ResourceType) => void
}

/** Create or edit a type: keys (canonical first, then aliases) and closed attributes. */
export const ResourceTypeSheet = ({ type, onClose }: Props) => {
  const invalidate = useInvalidateResources()
  const form = useZodForm({ schema, defaultValues: toForm(type) })
  const attributes = useFieldArray({ control: form.control, name: 'attributes' })
  const [name, keys] = useWatch({ control: form.control, name: ['name', 'keys'] })
  const [confirmClose, setConfirmClose] = useState(false)

  useEffect(() => {
    if (!type && !form.getFieldState('code').isDirty) form.setValue('code', slugify(name ?? ''))
  }, [name]) // eslint-disable-line react-hooks/exhaustive-deps

  const onError = (err: Error) => form.setError('root', { message: err.message.replace(/^\[\w+\]\s*/, '') })
  const create = useMutation(createResourceType, { onError })
  const update = useMutation(updateResourceType, { onError })

  const onSubmit = form.handleSubmit(async data => {
    const keyList = data.keys.map(k => k.trim())
    // The old canonical key stays an alias, so events and checks that used it keep resolving.
    if (type && !keyList.includes(type.keys[0])) {
      keyList.push(type.keys[0])
      toast.info(`${type.keys[0]} is kept as an alias`)
    }
    const attrs = Object.fromEntries(data.attributes.map(a => [a.name.trim(), { values: a.values }]))
    const saved = type
      ? await update.mutateAsync({
          idOrCode: type.id,
          name: data.name,
          pluralName: data.pluralName,
          keys: { keys: keyList },
          attributes: { attributes: attrs },
        })
      : await create.mutateAsync({
          code: data.code,
          name: data.name,
          pluralName: data.pluralName,
          keys: keyList,
          attributes: attrs,
        })
    await invalidate()
    onClose(saved.resourceType)
  })

  const safeClose = () => (form.formState.isDirty ? setConfirmClose(true) : onClose())
  const canonical = keys?.[0]?.trim()
  const pending = create.isPending || update.isPending
  const attributesError =
    form.formState.errors.attributes?.message ?? form.formState.errors.attributes?.root?.message

  return (
    <>
      <Sheet open onOpenChange={open => !open && safeClose()}>
        <SheetContent size="medium" className="p-0">
          <Form {...form}>
            <form onSubmit={onSubmit} className="flex h-full w-full flex-col">
              <SheetHeader className="ml-6 mt-4">
                <SheetTitle>{type ? `Edit ${type.name}` : 'New resource type'}</SheetTitle>
              </SheetHeader>

              <div className="flex-1 overflow-y-auto">
                <div className="space-y-6 p-6">
                  <div className="space-y-4">
                    <div className="grid grid-cols-2 gap-3">
                      <InputFormField name="name" label="Name" placeholder="Site" required control={form.control} />
                      <InputFormField
                        name="pluralName"
                        label="Plural name"
                        placeholder="Sites"
                        required
                        control={form.control}
                      />
                    </div>
                    <InputFormField
                      name="code"
                      label="Code"
                      placeholder="site"
                      required
                      disabled={!!type}
                      className="font-mono"
                      control={form.control}
                    />
                    <GenericFormField
                      name="keys"
                      label="Identified by"
                      required
                      control={form.control}
                      labelTooltip={<Hint>The first key is canonical; the others are aliases.</Hint>}
                      render={({ field }) => (
                        <ValuesTagInput value={field.value} onChange={field.onChange} placeholder="site_id" />
                      )}
                    />
                  </div>

                  <div className="space-y-4">
                    <div className="flex items-center justify-between gap-2">
                      <h3 className="font-semibold">Attributes</h3>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        hasIcon
                        onClick={() => attributes.append({ name: '', values: [] })}
                      >
                        <PlusIcon size={14} /> Add attribute
                      </Button>
                    </div>
                    {attributes.fields.map((f, i) => (
                      <div key={f.id} className="flex items-start gap-2">
                        <InputFormField
                          name={`attributes.${i}.name`}
                          placeholder="environment"
                          aria-label="Attribute code"
                          containerClassName="w-40 shrink-0"
                          className="font-mono"
                          control={form.control}
                        />
                        <GenericFormField
                          name={`attributes.${i}.values`}
                          control={form.control}
                          containerClassName="min-w-0 flex-1"
                          render={({ field }) => (
                            <ValuesTagInput value={field.value} onChange={field.onChange} placeholder="Allowed values" />
                          )}
                        />
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          aria-label="Remove attribute"
                          onClick={() => attributes.remove(i)}
                        >
                          <XIcon size={14} />
                        </Button>
                      </div>
                    ))}
                    {attributesError && <p className="text-sm text-destructive">{attributesError}</p>}
                  </div>

                  {type && canonical && canonical !== type.keys[0] && (
                    <TypeCoverage type={type} canonical={canonical} />
                  )}

                  {form.formState.errors.root?.message && (
                    <p className="text-sm text-destructive">{form.formState.errors.root.message}</p>
                  )}
                </div>
              </div>

              <SheetFooter className="border-t border-border p-3">
                <Button type="button" variant="outline" onClick={safeClose}>
                  Cancel
                </Button>
                <Button type="submit" disabled={pending}>
                  {type ? 'Save' : 'Create resource type'}
                </Button>
              </SheetFooter>
            </form>
          </Form>
        </SheetContent>
      </Sheet>

      <ConfirmationModal
        visible={confirmClose}
        danger
        header="Discard unsaved changes?"
        buttonLabel="Discard"
        onSelectCancel={() => setConfirmClose(false)}
        onSelectConfirm={() => {
          setConfirmClose(false)
          onClose()
        }}
      />
    </>
  )
}

const Hint = ({ children }: { children: string }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <InfoIcon className="h-3.5 w-3.5 cursor-help text-muted-foreground" />
    </TooltipTrigger>
    <TooltipContent className="max-w-72">{children}</TooltipContent>
  </Tooltip>
)
