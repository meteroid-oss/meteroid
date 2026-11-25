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

import { AdvancedSection, useHasFieldErrors } from '@/components/form'
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

import type { OptionLike } from '@/features/plans/currencies/priceCells'

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

// Select items can't carry an empty value.
const DEFAULT_OPTION = '__default__'

const schema = z.object({
  option: z.string(),
  name: z.string().trim().min(1, 'Name is required').max(200),
  allowCouponCodes: z.boolean(),
  couponCode: z.string().trim().max(64),
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
  options,
  showListed,
  planIssuesLicense,
  onClose,
}: {
  planId: string
  link?: PurchaseLink
  /** The options the plan's active version sells. */
  options: OptionLike[]
  showListed: boolean
  planIssuesLicense: boolean
  onClose: () => void
}) => {
  const queryClient = useQueryClient()
  const methods = useZodForm({
    schema,
    defaultValues: {
      option: link?.option ?? DEFAULT_OPTION,
      name: link?.name ?? '',
      allowCouponCodes: link?.allowCouponCodes ?? false,
      couponCode: link?.couponCode ?? '',
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
  const [chosenOption, couponCode] = useWatch({
    control: methods.control,
    name: ['option', 'couponCode'],
  })
  const moreInvalid = useHasFieldErrors(methods.control, ['option', 'couponCode'])
  const showOption = options.length > 1 || link?.option !== undefined
  const soldOption =
    showOption && chosenOption !== DEFAULT_OPTION
      ? (options.find(o => o.alias === chosenOption)?.name ?? chosenOption)
      : undefined
  const moreSummary = [soldOption, couponCode.trim() || undefined]
    .filter(Boolean)
    .join(' · ')

  const onSuccess = () => {
    void invalidatePurchaseLinks(queryClient)
    toast.success(link ? 'Purchase link updated' : 'Purchase link created')
    onClose()
  }
  const onError = (err: { rawMessage?: string }) => {
    // A refused coupon is shown next to its code.
    if (
      err.rawMessage &&
      /coupon/i.test(err.rawMessage) &&
      methods.getValues('couponCode').trim()
    ) {
      methods.setError('couponCode', { message: err.rawMessage })
      return
    }
    toast.error(err.rawMessage || 'Could not save the purchase link')
  }

  const create = useMutation(createPurchaseLink, { onSuccess, onError })
  const update = useMutation(updatePurchaseLink, { onSuccess, onError })
  const isPending = create.isPending || update.isPending

  const onSubmit = (data: z.infer<typeof schema>) => {
    const fields = {
      name: data.name,
      allowCouponCodes: data.allowCouponCodes,
      couponCode: data.couponCode || undefined,
      listed: data.listed,
      successUrl: data.successUrl || undefined,
      cancelUrl: data.cancelUrl || undefined,
      allowedBuyerTypes: AllowedBuyerTypes[data.allowedBuyerTypes],
      requireBillingAddress: data.requireBillingAddress,
      taxIdCollection: TaxIdCollection[data.taxIdCollection],
      option: data.option === DEFAULT_OPTION ? undefined : data.option,
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
            <AdvancedSection
              label="More options"
              summary={moreSummary}
              defaultOpen={link?.option !== undefined || !!link?.couponCode}
              forceOpen={moreInvalid}
            >
              {showOption && (
                <SelectFormField
                  name="option"
                  label="Option"
                  control={methods.control}
                  labelTooltip="What buyers subscribe to, among the plan's options."
                >
                  <SelectItem value={DEFAULT_OPTION}>Default</SelectItem>
                  {options.map(option => (
                    <SelectItem key={option.id} value={option.alias}>
                      {option.name}
                    </SelectItem>
                  ))}
                  {link?.option !== undefined && !options.some(o => o.alias === link.option) && (
                    <SelectItem value={link.option}>{link.option} (no longer sold)</SelectItem>
                  )}
                </SelectFormField>
              )}
              <InputFormField
                name="couponCode"
                label="Coupon code"
                control={methods.control}
                placeholder="e.g. LAUNCH20"
                autoComplete="off"
                description="Applied to every purchase from this link unless the buyer enters another, even when buyers can't enter codes."
              />
            </AdvancedSection>
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
