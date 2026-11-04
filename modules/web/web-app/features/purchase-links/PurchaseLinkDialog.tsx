import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
  SelectFormField,
  SelectItem,
  SwitchFormField,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useWatch } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { invalidatePurchaseLinks } from '@/features/purchase-links/utils'
import { useZodForm } from '@/hooks/useZodForm'
import {
  createPurchaseLink,
  updatePurchaseLink,
} from '@/rpc/api/purchaselinks/v1/purchaselinks-PurchaseLinksService_connectquery'
import {
  AllowedBuyerTypes,
  PurchaseLink,
  TaxIdCollection,
} from '@/rpc/api/purchaselinks/v1/purchaselinks_pb'

const isHttpUrl = (value: string) => {
  try {
    const url = new URL(value)
    return url.protocol === 'https:' || url.protocol === 'http:'
  } catch {
    return false
  }
}

const redirectUrl = z
  .string()
  .trim()
  .max(2048)
  .refine(v => v === '' || isHttpUrl(v), 'Enter an absolute http(s) URL')

const buyerTypeKeys = ['ALL', 'COMPANIES_ONLY', 'INDIVIDUALS_ONLY'] as const

const buyerTypeLabels = {
  ALL: 'Companies and individuals',
  COMPANIES_ONLY: 'Companies only',
  INDIVIDUALS_ONLY: 'Individuals only',
} satisfies Record<keyof typeof AllowedBuyerTypes, string>

const taxIdKeys = ['OFF', 'OPTIONAL', 'REQUIRED'] as const

const taxIdLabels = {
  OFF: 'Not asked',
  OPTIONAL: 'Optional',
  REQUIRED: 'Required',
} satisfies Record<keyof typeof TaxIdCollection, string>

const schema = z.object({
  name: z.string().trim().min(1, 'Name is required').max(200),
  allowCouponCodes: z.boolean(),
  listed: z.boolean(),
  successUrl: redirectUrl,
  cancelUrl: redirectUrl,
  allowedBuyerTypes: z.enum(buyerTypeKeys),
  requireBillingAddress: z.boolean(),
  taxIdCollection: z.enum(taxIdKeys),
})

export const PurchaseLinkDialog = ({
  planId,
  link,
  showListed,
  planIssuesLicense,
  onClose,
}: {
  planId: string
  link?: PurchaseLink
  showListed: boolean
  planIssuesLicense: boolean
  onClose: () => void
}) => {
  const queryClient = useQueryClient()
  const methods = useZodForm({
    schema,
    defaultValues: {
      name: link?.name ?? '',
      allowCouponCodes: link?.allowCouponCodes ?? false,
      listed: link?.listed ?? false,
      successUrl: link?.successUrl ?? '',
      cancelUrl: link?.cancelUrl ?? '',
      allowedBuyerTypes:
        buyerTypeKeys.find(key => AllowedBuyerTypes[key] === link?.allowedBuyerTypes) ?? 'ALL',
      requireBillingAddress: link?.requireBillingAddress ?? true,
      taxIdCollection:
        taxIdKeys.find(key => TaxIdCollection[key] === link?.taxIdCollection) ?? 'OPTIONAL',
    },
  })

  const listed = useWatch({ control: methods.control, name: 'listed' })
  const allowedBuyerTypes = useWatch({ control: methods.control, name: 'allowedBuyerTypes' })

  const onSuccess = () => {
    void invalidatePurchaseLinks(queryClient)
    toast.success(link ? 'Purchase link updated' : 'Purchase link created')
    onClose()
  }
  const onError = (err: { rawMessage?: string }) =>
    toast.error(err.rawMessage || 'Could not save the purchase link')

  const create = useMutation(createPurchaseLink, { onSuccess, onError })
  const update = useMutation(updatePurchaseLink, { onSuccess, onError })
  const isPending = create.isPending || update.isPending

  const onSubmit = (data: z.infer<typeof schema>) => {
    const fields = {
      name: data.name,
      allowCouponCodes: data.allowCouponCodes,
      listed: data.listed,
      successUrl: data.successUrl || undefined,
      cancelUrl: data.cancelUrl || undefined,
      allowedBuyerTypes: AllowedBuyerTypes[data.allowedBuyerTypes],
      requireBillingAddress: data.requireBillingAddress,
      taxIdCollection: TaxIdCollection[data.taxIdCollection],
    }
    if (link) update.mutate({ id: link.id, ...fields })
    else create.mutate({ planId, ...fields })
  }

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{link ? 'Edit purchase link' : 'New purchase link'}</DialogTitle>
          <DialogDescription>
            A public URL anyone can use to buy this plan: it creates the customer and sends them to
            checkout.
          </DialogDescription>
        </DialogHeader>
        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
            <InputFormField
              name="name"
              label="Name"
              control={methods.control}
              placeholder="e.g. Website pricing page"
              description="Internal, to tell your links apart."
            />
            <InputFormField
              name="successUrl"
              label="Success URL"
              control={methods.control}
              placeholder="https://example.com/welcome"
              description="Where buyers land after paying. Leave empty for the default confirmation page."
            />
            <InputFormField
              name="cancelUrl"
              label="Cancel URL"
              control={methods.control}
              placeholder="https://example.com/pricing"
              description="Where buyers go if they leave the checkout."
            />
            <SelectFormField name="allowedBuyerTypes" label="Buyers" control={methods.control}>
              {buyerTypeKeys.map(value => (
                <SelectItem key={value} value={value}>
                  {buyerTypeLabels[value]}
                </SelectItem>
              ))}
            </SelectFormField>

            {allowedBuyerTypes !== 'INDIVIDUALS_ONLY' && (
                <SelectFormField
                    name="taxIdCollection"
                    label="Company tax ID"
                    control={methods.control}
                >
                  {taxIdKeys.map(value => (
                      <SelectItem key={value} value={value}>
                        {taxIdLabels[value]}
                      </SelectItem>
                  ))}
                </SelectFormField>
            )}
            <SwitchFormField
              name="requireBillingAddress"
              control={methods.control}
              label="Require a billing address"
              description="Buyers enter their full address. When off, only their country is required, unless the default invoicing entity requires billing information at checkout."
            />

            <SwitchFormField
              name="allowCouponCodes"
              control={methods.control}
              label="Allow coupon codes"
              description="Buyers can enter a coupon code at checkout."
            />
            {showListed && (
              <SwitchFormField
                name="listed"
                control={methods.control}
                label="Offer in licensed software"
                disabled={!planIssuesLicense && !listed}
                description={
                  planIssuesLicense
                    ? 'Licensed installations list this link as a way to subscribe, and get their license once the purchase completes.'
                    : 'Only for plans that issue licenses: turn on "Issues a license" in the plan settings first.'
                }
              />
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" disabled={isPending}>
                {isPending ? 'Saving...' : link ? 'Save' : 'Create link'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
