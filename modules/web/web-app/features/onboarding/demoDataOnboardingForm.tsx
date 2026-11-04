import { useMutation } from '@connectrpc/connect-query'
import { Button, Flex, Spinner } from '@md/ui'
import { useNavigate } from 'react-router-dom'

import { buildSyncParam } from '@/hooks/useSyncQueries'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { createTenant } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'
import { SandboxPreset } from '@/rpc/api/tenants/v1/tenants_pb'

interface DemoDataOnboardingFormProps {
  organizationSlug: string
}

export const DemoDataOnboardingForm = ({ organizationSlug }: DemoDataOnboardingFormProps) => {
  const navigate = useNavigate()

  const createTenantMut = useMutation(createTenant, {
    onSuccess: ({ tenant }) => {
      if (tenant) {
        navigate(`/${organizationSlug}/${tenant.slug}?${buildSyncParam('stats')}`)
      }
    },
  })

  const addDemoData = () =>
    createTenantMut.mutate({
      name: 'Sandbox',
      environment: TenantEnvironmentEnum.SANDBOX,
      sandboxPreset: SandboxPreset.DEFAULT,
    })

  return (
    <Flex
      direction="column"
      className="w-full h-full gap-2 p-6 sm:p-8 md:p-10 lg:p-[52px] overflow-y-auto"
    >
      <div className="font-medium text-xl -mb-0.5">Explore with demo data?</div>
      <div className="text-muted-foreground text-[13px] mb-5 leading-[18px]">
        We can add a Sandbox tenant filled with a sample business: plans, customers, subscriptions,
        invoices and usage. It stays apart from your own tenants, and you can delete it whenever you
        like.
      </div>

      <Flex direction="column" className="gap-2 mt-auto">
        <Button
          variant="primary"
          hasIcon={createTenantMut.isPending}
          disabled={createTenantMut.isPending}
          onClick={addDemoData}
        >
          {createTenantMut.isPending && <Spinner />}
          Add demo data
        </Button>
        <Button
          variant="ghost"
          disabled={createTenantMut.isPending}
          onClick={() => navigate(`/${organizationSlug}`)}
        >
          Skip
        </Button>
      </Flex>
    </Flex>
  )
}
