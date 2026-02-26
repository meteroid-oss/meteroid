import { Button, Card, Checkbox, Flex, Separator } from '@md/ui'
import { Heart, SettingsIcon } from 'lucide-react'
import { useMemo } from 'react'
import { Link } from 'react-router-dom'

import { Loading } from '@/components/Loading'
import { DetailsSection } from '@/features/dashboard/sections/DetailsSection'
import { MrrSection } from '@/features/dashboard/sections/MrrSection'
import { TopSection } from '@/features/dashboard/sections/TopSection'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useSyncQueries } from '@/hooks/useSyncQueries'
import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const Dashboard = () => {
  // Auto-refresh queries based on URL params (e.g., ?_sync=stats after onboarding)
  useSyncQueries()
  const { isRefetching } = useTenant()
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

  // morning, afternoon or evening /
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

  if (isRefetching) {
    return <Loading />
  }

  return (
    <>
      <div className="h-full  w-full self-center space-y-6 relative">
        <div>
          <h1 className="text-2xl text-acc font-semibold">
            Good {timeOfDay}
            {username ? `, ${username}` : null}
          </h1>
          <span className="text-md font-medium text-muted-foreground">{date}</span>
        </div>
        <Separator />
        {showAddressSetup && (
          <Card variant="accent2">
            <div className="px-6 py-4">
              <div className="flex items-center justify-between">
                <div>
                  <div className="text-sm font-semibold pb-1">Complete your setup</div>
                  <div className="text-xs text-muted-foreground">
                    Add your business address to start generating invoices
                  </div>
                </div>
                <Button variant="primary" size="sm" hasIcon asChild>
                  <Link to="settings?tab=merchant">
                    <SettingsIcon size={14} />
                    Configure
                  </Link>
                </Button>
              </div>
            </div>
          </Card>
        )}
        <Card variant="accent2" className="hidden">
          <div className="px-6 py-4">
            <div className="text-sm font-semibold pb-4">Complete your onboarding</div>
            <Flex direction="column" className="gap-2">
              <Flex align="center" className="gap-2">
                <Checkbox disabled className="rounded-full border-none bg-success" checked />{' '}
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
        <MrrSection />
        <Separator />
        <DetailsSection />
        <Separator />
        <div className="h-10 text-center justify-center text-xs text-muted-foreground flex gap-1 ">
          <span>2025 © Meteroid /</span>
          <span className="flex items-baseline gap-1">
            Built with <Heart size="10" fill="red" strokeWidth={0} className="" /> in Europe /
          </span>
          <span>
            Open-source on{' '}
            <a href="https://go.meteroid.com/github" className="underline">
              Github
            </a>
          </span>
        </div>
      </div>
    </>
  )
}
