import {
  cn,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Flex,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  useSidebar,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { PlusIcon, Undo2Icon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Link } from 'react-router'

import { SelectorChevronsIcon } from '@/components/icons/AppIcons'
import {
  SIDEBAR_MENU_CONTENT,
  SIDEBAR_MENU_ITEM,
  SIDEBAR_MENU_ITEM_CURRENT,
  SIDEBAR_MENU_SEPARATOR,
} from '@/components/layouts/shared/sidebarMenuStyles'
import {
  useCancelTenantDeletion,
  usePendingDeletionTenants,
} from '@/features/settings/PendingDeletionTenants'
import { CreateTenantModal } from '@/features/tenants/CreateTenant'
import { useOrganizationSlug } from '@/hooks/useOrganization'
import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const getColor = (environment: TenantEnvironmentEnum | undefined) => {
  switch (environment) {
    case TenantEnvironmentEnum.PRODUCTION:
      return 'bg-cyan-600'
    default:
      return 'bg-yellow-500'
  }
}

export const TenantDropdown = () => {
  const { tenant } = useTenant()
  const org = useOrganizationSlug()
  const { state } = useSidebar()

  const queryClient = useQueryClient()

  const tenants = useQuery(listTenants).data?.tenants ?? []
  const pendingDeletion = usePendingDeletionTenants()
  const restoreMut = useCancelTenantDeletion({ openOnSuccess: true })

  const [open, setOpen] = useState(false)
  const [creating, setCreating] = useState(false)

  useEffect(() => {
    setOpen(false)
  }, [tenant])

  // The environment's colour in the round slot the design gives the tenant (as in the menu below).
  const tenantColor = (
    <span className="flex size-4 shrink-0 items-center justify-center rounded-full bg-foreground/10">
      <span className={cn('size-1.5 rounded-full', getColor(tenant?.environment))} />
    </span>
  )

  const menu =
    state === 'collapsed' ? (
      <Tooltip>
        <TooltipTrigger asChild>
          <div className="flex h-8 w-full items-center justify-center">
            <span className={`size-2 rounded-full ${getColor(tenant?.environment)}`} />
          </div>
        </TooltipTrigger>
        <TooltipContent side="right">{tenant?.name}</TooltipContent>
      </Tooltip>
    ) : (
      <DropdownMenu open={open} onOpenChange={setOpen}>
        <DropdownMenuTrigger className="w-full rounded-full outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring">
          {/* A pill: a quiet 6% tint (about #222 on the sidebar) and a 10% border. */}
          <Flex
            align="center"
            className="h-8 w-full gap-2 rounded-full border border-foreground/10 bg-foreground/[0.06] pl-2.5 pr-4 transition-colors hover:bg-foreground/10"
          >
            {tenantColor}
            <span
              className="min-w-0 flex-1 truncate text-left text-[13px] leading-[18px] font-medium text-foreground"
              title={tenant?.name}
            >
              {tenant?.name}
            </span>
            <SelectorChevronsIcon className="text-faint-foreground" />
          </Flex>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          side="top"
          align="start"
          className={cn('max-h-80 w-[246px] overflow-y-auto', SIDEBAR_MENU_CONTENT)}
        >
          {[...tenants]
            .sort((a, b) => a.name.localeCompare(b.name))
            .map(t => (
              <DropdownMenuItem
                key={t.id}
                asChild
                className={cn(SIDEBAR_MENU_ITEM, t.id === tenant?.id && SIDEBAR_MENU_ITEM_CURRENT)}
              >
                <Link to={`/${org}/${t.slug}`} onClick={() => queryClient.invalidateQueries()}>
                  {/* The environment's colour, in the round slot the design gives each tenant. */}
                  <span className="flex size-4 shrink-0 items-center justify-center rounded-full bg-foreground/10">
                    <span className={cn('size-1.5 rounded-full', getColor(t.environment))} />
                  </span>
                  <span className="truncate">{t.name}</span>
                </Link>
              </DropdownMenuItem>
            ))}
          <DropdownMenuSeparator className={SIDEBAR_MENU_SEPARATOR} />
          <DropdownMenuItem className={SIDEBAR_MENU_ITEM} onSelect={() => setCreating(true)}>
            <PlusIcon size={16} strokeWidth={1.5} className="shrink-0" />
            Create new tenant
          </DropdownMenuItem>
          {pendingDeletion.length > 0 && (
            <>
              <DropdownMenuSeparator className={SIDEBAR_MENU_SEPARATOR} />
              <DropdownMenuLabel className="px-2 py-1 text-[11px] font-normal text-muted-foreground">
                Pending deletion
              </DropdownMenuLabel>
              {pendingDeletion.map(
                ({ tenant: deleted }) =>
                  deleted && (
                    <DropdownMenuItem
                      key={deleted.id}
                      className={SIDEBAR_MENU_ITEM}
                      disabled={restoreMut.isPending}
                      onSelect={() => restoreMut.mutate({ tenantId: deleted.id })}
                    >
                      <span className="min-w-0 flex-1 truncate">{deleted.name}</span>
                      <span className="flex shrink-0 items-center gap-1 text-xs">
                        <Undo2Icon size={12} /> Restore
                      </span>
                    </DropdownMenuItem>
                  )
              )}
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    )

  return (
    <>
      {menu}
      <CreateTenantModal open={creating} onOpenChange={setCreating} />
    </>
  )
}
