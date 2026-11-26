import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { toast } from 'sonner'
import { z } from 'zod'

import { humanize, keyLabel, lower, useInvalidateResources } from '@/features/resources/hooks'
import { useZodForm } from '@/hooks/useZodForm'
import { upsertResource } from '@/rpc/api/resources/v1/resources-ResourcesService_connectquery'

import type { Resource, ResourceType } from '@/rpc/api/resources/v1/models_pb'

const NOT_SET = '__not_set__'

const schema = z.object({
  keyValue: z.string().trim().min(1, 'Required'),
  name: z.string().trim(),
  attributes: z.record(z.string(), z.string()),
})

/** Add a stored resource to a customer, or edit its name and attributes. The key value is fixed. */
export const ResourceDialog = ({
  customerId,
  type,
  resource,
  withAttributes = true,
  onClose,
}: {
  customerId: string
  type: ResourceType
  resource?: Resource
  withAttributes?: boolean
  onClose: (saved?: Resource) => void
}) => {
  const invalidate = useInvalidateResources()
  const form = useZodForm({
    schema,
    defaultValues: {
      keyValue: resource?.keyValue ?? '',
      name: resource?.name ?? '',
      attributes: { ...resource?.attributes },
    },
  })
  const upsert = useMutation(upsertResource, {
    onSuccess: async res => {
      await invalidate()
      onClose(res.resource)
    },
    onError: err => toast.error(err.message),
  })
  const attributes = Object.entries(type.attributes).sort(([a], [b]) => a.localeCompare(b))

  const onSubmit = form.handleSubmit(data =>
    upsert.mutate({
      customerId,
      resourceType: type.id,
      keyValue: data.keyValue,
      name: data.name || undefined,
      // Full replacement: an omitted attribute is cleared.
      attributes: Object.fromEntries(
        Object.entries(data.attributes).filter(([, v]) => v && v !== NOT_SET)
      ),
      archived: resource?.archived ?? false,
    })
  )

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>
            {resource ? `Edit ${resource.name || resource.keyValue}` : `New ${lower(type.name)}`}
          </DialogTitle>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={onSubmit} className="space-y-4">
            <InputFormField
              name="keyValue"
              label={keyLabel(type)}
              rightLabel={<span className="font-mono text-xs text-muted-foreground">{type.keys[0]}</span>}
              required
              disabled={!!resource}
              className="font-mono"
              control={form.control}
            />
            <InputFormField name="name" label="Name" placeholder="Optional" control={form.control} />
            {withAttributes &&
              attributes.map(([attribute, { values }]) => (
                <SelectFormField
                  key={attribute}
                  name={`attributes.${attribute}`}
                  label={humanize(attribute)}
                  placeholder="Not set"
                  control={form.control}
                >
                  <SelectItem value={NOT_SET}>Not set</SelectItem>
                  {values.map(v => (
                    <SelectItem key={v} value={v}>
                      {v}
                    </SelectItem>
                  ))}
                </SelectFormField>
              ))}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onClose()}>
                Cancel
              </Button>
              <Button type="submit" disabled={upsert.isPending}>
                {resource ? 'Save' : `Add ${lower(type.name)}`}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
