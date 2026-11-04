import { useMutation } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Label,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ArchiveIcon, MoreHorizontalIcon, PencilIcon, RefreshCwIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { PageSection } from '@/components/layouts/shared/PageSection'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { PurchaseLinkDialog } from '@/features/purchase-links/PurchaseLinkDialog'
import { invalidatePurchaseLinks } from '@/features/purchase-links/utils'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import {
  archivePurchaseLink,
  listPurchaseLinks,
  regeneratePurchaseLinkUrl,
} from '@/rpc/api/purchaselinks/v1/purchaselinks-PurchaseLinksService_connectquery'
import { AllowedBuyerTypes, PurchaseLink } from '@/rpc/api/purchaselinks/v1/purchaselinks_pb'
import { parseAndFormatDate } from '@/utils/date'

type OpenDialog =
  | { kind: 'edit'; link?: PurchaseLink }
  | { kind: 'regenerate'; link: PurchaseLink }
  | { kind: 'archive'; link: PurchaseLink }

export const PurchaseLinksSection = () => {
  const { plan } = usePlanWithVersion()
  const isDraft = useIsDraftVersion()

  if (!plan || isDraft || plan.planType === PlanType.CUSTOM) return null
  return (
    <PurchaseLinks
      planId={plan.id}
      planActive={plan.planStatus === PlanStatus.ACTIVE}
      planIssuesLicense={plan.issuesLicense}
    />
  )
}

const PurchaseLinks = ({
  planId,
  planActive,
  planIssuesLicense,
}: {
  planId: string
  planActive: boolean
  planIssuesLicense: boolean
}) => {
  const queryClient = useQueryClient()
  const canManage = usePermissions().has('catalog:manage')
  const licensingEnabled = useEntitlement(entitlementCodes.LICENSING)
  const [includeArchived, setIncludeArchived] = useState(false)
  const [dialog, setDialog] = useState<OpenDialog>()
  const close = () => setDialog(undefined)

  const query = useQuery(listPurchaseLinks, { planId, includeArchived })
  const links = query.data?.links ?? []

  const onError = (fallback: string) => (err: { rawMessage?: string }) =>
    toast.error(err.rawMessage || fallback)

  const regenerate = useMutation(regeneratePurchaseLinkUrl, {
    onSuccess: () => {
      void invalidatePurchaseLinks(queryClient)
      toast.success('New URL issued')
      close()
    },
    onError: onError('Could not regenerate the URL'),
  })

  const archive = useMutation(archivePurchaseLink, {
    onSuccess: () => {
      void invalidatePurchaseLinks(queryClient)
      toast.success('Purchase link archived')
      close()
    },
    onError: onError('Could not archive the purchase link'),
  })

  const pendingLink =
    dialog?.kind === 'regenerate' || dialog?.kind === 'archive' ? dialog.link : undefined

  return (
    <PageSection
      header={{
        title: 'Purchase links',
        subtitle: 'Public, reusable links to buy this plan without a pre-existing account.',
        actions:
          canManage && planActive ? (
            <Button
              variant="outline"
              className="py-1.5"
              onClick={() => setDialog({ kind: 'edit' })}
            >
              + New purchase link
            </Button>
          ) : null,
      }}
    >
      <div className="flex flex-col gap-3">
        {(links.length > 0 || includeArchived) && (
          <div className="flex items-center gap-2">
            <Switch
              id="purchase-links-archived"
              checked={includeArchived}
              onCheckedChange={setIncludeArchived}
            />
            <Label
              htmlFor="purchase-links-archived"
              className="text-xs text-muted-foreground font-normal"
            >
              Show archived
            </Label>
          </div>
        )}

        {query.isLoading ? null : links.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border p-6 text-sm text-muted-foreground">
            <p>
              Share a purchase link on your website or in an email: buyers enter their email and
              billing details, then pay through checkout. Each purchase creates a new customer and a
              subscription to this plan.
            </p>
            {!planActive && <p className="mt-2">Publish the plan to create purchase links.</p>}
          </div>
        ) : (
          <div className="overflow-x-auto rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>URL</TableHead>
                  <TableHead className="text-right whitespace-nowrap">Started</TableHead>
                  <TableHead className="text-right whitespace-nowrap">Completed</TableHead>
                  <TableHead className="whitespace-nowrap">Created</TableHead>
                  <TableHead className="w-px" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {links.map(link => (
                  <TableRow key={link.id} className={link.archivedAt ? 'opacity-60' : undefined}>
                    <TableCell className="text-sm">
                      <div className="flex items-center gap-2">
                        <span className="font-medium truncate max-w-[200px]" title={link.name}>
                          {link.name}
                        </span>
                        {link.listed && (
                          <Badge variant="outline" size="sm">
                            Listed
                          </Badge>
                        )}
                        {link.allowCouponCodes && (
                          <Badge variant="outline" size="sm">
                            Coupons
                          </Badge>
                        )}
                        {link.allowedBuyerTypes !== AllowedBuyerTypes.ALL && (
                          <Badge variant="outline" size="sm">
                            {link.allowedBuyerTypes === AllowedBuyerTypes.COMPANIES_ONLY
                              ? 'Companies'
                              : 'Individuals'}
                          </Badge>
                        )}
                        {link.archivedAt && (
                          <Badge variant="secondary" size="sm">
                            Archived
                          </Badge>
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="max-w-[280px]">
                      {link.archivedAt ? (
                        <span className="text-xs text-muted-foreground">—</span>
                      ) : (
                        <CopyToClipboardButton
                          text={link.url}
                          className="inline-block max-w-[240px] font-mono"
                        />
                      )}
                    </TableCell>
                    <TableCell className="text-right text-sm tabular-nums">
                      {link.startedCount}
                    </TableCell>
                    <TableCell className="text-right text-sm tabular-nums">
                      {link.completedCount}
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-xs text-muted-foreground">
                      {parseAndFormatDate(link.createdAt)}
                    </TableCell>
                    <TableCell className="w-px">
                      {canManage && !link.archivedAt && (
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button variant="ghost" size="icon" className="h-8 w-8">
                              <MoreHorizontalIcon size={16} />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onClick={() => setDialog({ kind: 'edit', link })}>
                              <PencilIcon size={14} className="mr-2" /> Edit
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              onClick={() => setDialog({ kind: 'regenerate', link })}
                            >
                              <RefreshCwIcon size={14} className="mr-2" /> Regenerate URL
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem
                              className="text-destructive focus:text-destructive"
                              onClick={() => setDialog({ kind: 'archive', link })}
                            >
                              <ArchiveIcon size={14} className="mr-2" /> Archive
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>

      {dialog?.kind === 'edit' && (
        <PurchaseLinkDialog
          planId={planId}
          link={dialog.link}
          showListed={licensingEnabled || !!dialog.link?.listed}
          planIssuesLicense={planIssuesLicense}
          onClose={close}
        />
      )}

      <AlertDialog open={!!pendingLink} onOpenChange={open => !open && close()}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {dialog?.kind === 'archive' ? 'Archive purchase link?' : 'Regenerate URL?'}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {dialog?.kind === 'archive'
                ? 'The link stops working and is hidden from licensed software. Purchases already started can still complete.'
                : 'The link gets a new URL and the current one stops working right away. Update every place it is shared.'}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={regenerate.isPending || archive.isPending}
              onClick={e => {
                e.preventDefault()
                if (!pendingLink) return
                if (dialog?.kind === 'archive') archive.mutate({ id: pendingLink.id })
                else regenerate.mutate({ id: pendingLink.id })
              }}
            >
              {dialog?.kind === 'archive' ? 'Archive' : 'Regenerate'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </PageSection>
  )
}
