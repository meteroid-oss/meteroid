import { useSeedingStatus } from '@/hooks/useSeedingStatus'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

interface SeedingBannerProps {
  tenantId: string | undefined
  environment: TenantEnvironmentEnum | undefined
}

/** Shown while a seeded sandbox is still generating its historical data, or when it stopped. */
export const SeedingBanner = ({ tenantId, environment }: SeedingBannerProps) => {
  const isSandbox = environment === TenantEnvironmentEnum.SANDBOX
  const { inProgress, failed, failureReason } = useSeedingStatus(isSandbox ? tenantId : undefined)

  if (!isSandbox) {
    return null
  }

  if (failed) {
    return (
      <div className="absolute top-6 left-0 right-0 z-20 flex justify-center pointer-events-none">
        <div className="bg-destructive/10 border border-destructive/50 text-destructive text-xs font-semibold px-4 py-1 rounded-lg shadow-sm pointer-events-auto flex items-center gap-2">
          <span className="inline-block w-2 h-2 rounded-full bg-destructive" />
          {failureReason ?? 'Seeding your sandbox data stopped before it finished.'}
        </div>
      </div>
    )
  }

  if (!inProgress) {
    return null
  }

  return (
    <div className="absolute top-6 left-0 right-0 z-20 flex justify-center pointer-events-none">
      <div className="bg-muted text-white text-xs font-semibold px-4 py-1 rounded-lg shadow-sm pointer-events-auto flex items-center gap-2">
        <span className="inline-block w-2 h-2 rounded-full bg-white/80 animate-pulse" />
        Seeding your sandbox data… This can take a few minutes.
      </div>
    </div>
  )
}
