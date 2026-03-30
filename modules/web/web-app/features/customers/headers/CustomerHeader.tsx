import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  cn,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Flex as NewFlex,
  Separator,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Check, ChevronDown, ChevronRight } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { Loader } from '@/features/auth/components/Loader'
import {
  IntegrationType,
  SyncCustomerModal,
} from '@/features/settings/integrations/SyncCustomerModal'
import { useBasePath } from '@/hooks/useBasePath'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useQuery } from '@/lib/connectrpc'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  archiveCustomer,
  getCustomerById,
  listCustomers,
  unarchiveCustomer,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

interface CustomerHeaderProps {
  id?: string
  name?: string
  archivedAt?: Date
  setEditPanelVisible: (visible: boolean) => void
  setShowEditCustomer?: () => void
}

export const CustomerHeader: FunctionComponent<CustomerHeaderProps> = ({
  id,
  name,
  archivedAt,
  setShowEditCustomer,
}) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const isExpress = useIsExpressOrganization()

  const [search, setSearch] = useState('')
  const [showSyncHubspotModal, setShowSyncHubspotModal] = useState(false)
  const [showSyncPennylaneModal, setShowSyncPennylaneModal] = useState(false)
  const [showArchiveDialog, setShowArchiveDialog] = useState(false)

  const isArchived = Boolean(archivedAt)

  const debouncedSearch = useDebounceValue(search, 400)

  const pageIndex = 0
  const pageSize = 20

  const customersQuery = useQuery(
    listCustomers,
    {
      pagination: {
        perPage: pageSize,
        page: pageIndex,
      },
      search: debouncedSearch.length > 0 ? debouncedSearch : undefined,
    },
    {}
  )

  const data = customersQuery.data?.customers ?? []

  const connectorsQuery = useQuery(listConnectors, {})
  const connectorsData = connectorsQuery.data?.connectors ?? []

  const isHubspotConnected = connectorsData.some(
    connector => connector.provider === ConnectorProviderEnum.HUBSPOT
  )
  const isPennylaneConnected = connectorsData.some(
    connector => connector.provider === ConnectorProviderEnum.PENNYLANE
  )

  const isLoading = customersQuery.isLoading || connectorsQuery.isLoading

  // Query subscriptions to check if customer has active subscriptions
  const subscriptionsQuery = useQuery(
    listSubscriptions,
    {
      customerId: id ?? '',
      status: [SubscriptionStatus.PENDING, SubscriptionStatus.TRIALING, SubscriptionStatus.ACTIVE],
    },
    { enabled: Boolean(id) }
  )

  const hasActiveSubscriptions = (subscriptionsQuery.data?.subscriptions.length ?? 0) > 0

  const archiveCustomerMut = useMutation(archiveCustomer, {
    onSuccess: async () => {
      toast.success('Customer archived successfully')
      // Invalidate customer list cache
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listCustomers),
      })
      navigate(`${basePath}/customers`)
    },
    onError: error => {
      toast.error(`Failed to archive customer: ${error.message}`)
    },
  })

  const unarchiveCustomerMut = useMutation(unarchiveCustomer, {
    onSuccess: async () => {
      toast.success('Customer unarchived successfully')
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey(listCustomers),
        }),
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey(getCustomerById, { id }),
        }),
      ])
    },
    onError: error => {
      toast.error(`Failed to unarchive customer: ${error.message}`)
    },
  })

  const handleArchiveCustomer = async () => {
    if (!id) return
    await archiveCustomerMut.mutateAsync({ id })
    setShowArchiveDialog(false)
  }

  const handleUnarchiveCustomer = async () => {
    if (!id) return
    await unarchiveCustomerMut.mutateAsync({ id })
  }

  return (
    <>
      {showSyncHubspotModal && (
        <SyncCustomerModal
          name={name ?? ''}
          id={id ?? ''}
          integrationType={IntegrationType.Hubspot}
          onClose={() => setShowSyncHubspotModal(false)}
        />
      )}
      {showSyncPennylaneModal && (
        <SyncCustomerModal
          name={name ?? ''}
          id={id ?? ''}
          integrationType={IntegrationType.Pennylane}
          onClose={() => setShowSyncPennylaneModal(false)}
        />
      )}
      <Dialog open={showArchiveDialog} onOpenChange={setShowArchiveDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Archive Customer</DialogTitle>
            <DialogDescription>
              Are you sure you want to archive &quot;{name}&quot;?
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setShowArchiveDialog(false)}
              disabled={archiveCustomerMut.isPending}
            >
              Cancel
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={handleArchiveCustomer}
              disabled={archiveCustomerMut.isPending}
            >
              {archiveCustomerMut.isPending ? 'Archiving...' : 'Archive'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <div className="flex flex-col gap-2">
        <div className="flex flex-row items-center justify-between">
          <NewFlex align="center" className="gap-2">
            <img src="/header/customer.svg" alt="customer logo" />
            <div
              className="text-[15px] font-medium text-muted-foreground cursor-pointer"
              onClick={() => navigate('..')}
            >
              Customers
            </div>
            <ChevronRight size={14} className="text-muted-foreground" />
            <div className="text-[15px] font-medium">{name}</div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <NewFlex
                  align="center"
                  justify="center"
                  className="h-4 w-4 cursor-pointer rounded bg-[#323232]"
                >
                  <ChevronDown size={15} className="text-[#76777D]" />
                </NewFlex>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" alignOffset={-134} sideOffset={5} className="w-56">
                <Input
                  value={search}
                  onChange={e => setSearch(e.target.value)}
                  autoFocus
                  placeholder="Search..."
                  className="h-7 w-full bg-transparent focus-visible:shadow-none outline-none border-none"
                />
                <DropdownMenuSeparator />
                {isLoading ? (
                  <Loader />
                ) : (
                  data.map(customer => (
                    <DropdownMenuItem
                      onClick={() => navigate(`${basePath}/customers/${customer.id}`)}
                      key={customer.id}
                      className={cn(customer.name === name && 'bg-accent', 'mt-1 cursor-pointer')}
                    >
                      <NewFlex align="center" justify="between" className="w-full">
                        {customer.name}
                        {customer.name === name && <Check size={16} />}
                      </NewFlex>
                    </DropdownMenuItem>
                  ))
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </NewFlex>
          {!isExpress && (
            <div className="flex flex-row gap-2">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button hasIcon size="sm" variant="secondary">
                    Actions <ChevronDown size={14} className="text-muted-foreground" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-[241px]">
                  <DropdownMenuItem id="edit_customer" onClick={setShowEditCustomer}>
                    Edit Customer
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    id="create_invoice"
                    onClick={() => navigate(`${basePath}/invoices/create?customerId=${id}`)}
                  >
                    Create Invoice
                  </DropdownMenuItem>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          id="sync_to_hubspot"
                          disabled={!isHubspotConnected}
                          onClick={() => setShowSyncHubspotModal(true)}
                        >
                          Sync to Hubspot
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!isHubspotConnected && (
                      <TooltipContent>Hubspot integration not connected</TooltipContent>
                    )}
                  </Tooltip>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span>
                        <DropdownMenuItem
                          id="sync_to_pennylane"
                          disabled={!isPennylaneConnected}
                          onClick={() => setShowSyncPennylaneModal(true)}
                        >
                          Sync to Pennylane
                        </DropdownMenuItem>
                      </span>
                    </TooltipTrigger>
                    {!isPennylaneConnected && (
                      <TooltipContent>Pennylane integration not connected</TooltipContent>
                    )}
                  </Tooltip>
                  <DropdownMenuSeparator />
                  {isArchived ? (
                    <DropdownMenuItem
                      id="unarchive_customer"
                      onClick={handleUnarchiveCustomer}
                      disabled={unarchiveCustomerMut.isPending}
                    >
                      {unarchiveCustomerMut.isPending ? 'Unarchiving...' : 'Unarchive customer'}
                    </DropdownMenuItem>
                  ) : (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span>
                          <DropdownMenuItem
                            id="archive_customer"
                            disabled={hasActiveSubscriptions}
                            onClick={() => setShowArchiveDialog(true)}
                            className="text-destructive focus:text-destructive"
                          >
                            Archive customer
                          </DropdownMenuItem>
                        </span>
                      </TooltipTrigger>
                      {hasActiveSubscriptions && (
                        <TooltipContent>Customer has active subscriptions</TooltipContent>
                      )}
                    </Tooltip>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          )}
        </div>
        <div className="mx-[-16px]">
          <Separator />
        </div>
      </div>
    </>
  )
}
