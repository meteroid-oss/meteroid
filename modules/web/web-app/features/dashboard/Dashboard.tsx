import { Button, Card, Checkbox, Flex, Separator } from '@md/ui'
import { SettingsIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router'

import { QuickActions } from '@/features/dashboard/QuickActions'
import { DashboardCard } from '@/features/dashboard/cards/DashboardCard'
import { DetailsSection } from '@/features/dashboard/sections/DetailsSection'
import { MrrSection } from '@/features/dashboard/sections/MrrSection'
import { TopSection } from '@/features/dashboard/sections/TopSection'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useSyncQueries } from '@/hooks/useSyncQueries'
import { useQuery } from '@/lib/connectrpc'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const Dashboard = () => {
  // Auto-refresh queries based on URL params (e.g., ?_sync=stats after onboarding)
  useSyncQueries()
  const isExpress = useIsExpressOrganization()
  const { defaultEntity, isLoading: isLoadingEntity } = useInvoicingEntity()

  const username = useQuery(me)?.data?.user?.firstName

  const showAddressSetup = useMemo(() => {
    if (isLoadingEntity || !defaultEntity) return false
    return !defaultEntity.addressLine1 || !defaultEntity.city
  }, [defaultEntity, isLoadingEntity])

  const date = useMemo(() => {
    const today = new Date()
    const options = { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' } as const

    return today.toLocaleDateString('en-US', options)
  }, [])

  // morning, afternoon or evening
  const timeOfDay = useMemo(() => {
    const hour = new Date().getHours()
    if (hour > 18 || hour < 4) {
      return 'evening'
    } else if (hour > 12) {
      return 'afternoon'
    } else {
      return 'morning'
    }
  }, [])

  return (
    <>
      <div className="relative mx-auto flex w-full max-w-[1024px] flex-col gap-8 pt-4 pb-10">
        <div className="flex flex-col gap-3">
          <h1 className="font-display text-2xl leading-none font-medium">
            Good {timeOfDay}
            {username ? `, ${username}` : null}
          </h1>
          <span className="text-[13px] leading-none tracking-[-0.13px] text-foreground/60">
            {date}
          </span>
        </div>
        {!isExpress && <QuickActions />}
        <Separator className="bg-foreground/[0.07]" />
        {showAddressSetup && !isExpress && (
          <DashboardCard>
            <div>
              <div className="flex items-center justify-between">
                <div>
                  <div className="text-sm font-semibold pb-1">Complete your setup</div>
                  <div className="text-xs text-muted-foreground">
                    Add your business address to start generating invoices
                  </div>
                </div>
                <Button variant="primary" size="sm" hasIcon asChild>
                  <Link to="settings/merchant">
                    <SettingsIcon size={14} />
                    Configure
                  </Link>
                </Button>
              </div>
            </div>
          </DashboardCard>
        )}
        <Card variant="accent2" className="hidden">
          <div className="px-6 py-4">
            <div className="text-sm font-semibold pb-4">Complete your onboarding</div>
            <Flex direction="column" className="gap-2">
              <Flex align="center" className="gap-2">
                <Checkbox
                  disabled
                  className="rounded-full border-none bg-success before:hidden [--check-mark:white]"
                  checked
                />{' '}
                <div className="text-sm mt-[0.5px]">Configure your pricing</div>
              </Flex>
              <Flex align="center" className="gap-2">
                <Checkbox disabled className="rounded-full" />{' '}
                <div className="text-sm mt-[0.5px]">Integrate with your product</div>
              </Flex>

              <Flex align="center" className="gap-2">
                <Checkbox disabled className="rounded-full" />{' '}
                <div className="text-sm mt-[0.5px]">Setup your first growth opportunities</div>
              </Flex>
            </Flex>
          </div>
        </Card>
        <TopSection />
        {!isExpress && <MrrSection />}
        <Separator className="bg-foreground/[0.07]" />
        <DetailsSection />
      </div>
    </>
  )
}
