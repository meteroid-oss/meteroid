import { cn } from '@ui/index'
import { useCallback, useEffect, useRef, useState } from 'react'

import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

interface TestModeBannerProps {
  environment: TenantEnvironmentEnum | undefined
}

const environmentLabels: Record<TenantEnvironmentEnum, string> = {
  [TenantEnvironmentEnum.PRODUCTION]: 'Production',
  [TenantEnvironmentEnum.STAGING]: 'Staging',
  [TenantEnvironmentEnum.QA]: 'QA',
  [TenantEnvironmentEnum.DEVELOPMENT]: 'Development',
  [TenantEnvironmentEnum.SANDBOX]: 'Sandbox',
  [TenantEnvironmentEnum.DEMO]: 'Demo',
}

// Badge auto-hides shortly after load, leaving only the amber line as a
// persistent reminder. Hovering the top edge peeks the label back for a moment.
export const TestModeBanner = ({ environment }: TestModeBannerProps) => {
  const [revealed, setRevealed] = useState(true)
  const timeoutRef = useRef<ReturnType<typeof setTimeout>>()

  // Show the badge, then hide after a delay. Any prior timer is cleared, so
  // repeated calls (mount + hovers) never stack concurrent timers.
  const reveal = useCallback(() => {
    setRevealed(true)
    if (timeoutRef.current) clearTimeout(timeoutRef.current)
    timeoutRef.current = setTimeout(() => setRevealed(false), 2500)
  }, [])

  useEffect(() => {
    reveal()
    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current)
    }
  }, [environment, reveal])

  if (environment === undefined || environment === TenantEnvironmentEnum.PRODUCTION) {
    return null
  }

  const label = environmentLabels[environment] || 'Test'

  return (
    <div className="pointer-events-none absolute top-0 left-0 right-0 z-10 flex justify-center">
      {/* Persistent amber line, doubling as a thin hover zone to peek the badge */}
      <div
        className="pointer-events-auto absolute top-0 left-0 right-0 h-2"
        onMouseEnter={reveal}
      >
        <div className="absolute top-0 left-0 right-0 h-0.5 bg-amber-500" />
      </div>
      {/* Centered badge, fades out shortly after being revealed */}
      <div
        className={cn(
          'bg-amber-500 text-amber-950 text-xs font-semibold px-4 py-1 rounded-b-lg shadow-sm transition-opacity duration-500',
          revealed ? 'opacity-100' : 'opacity-0'
        )}
      >
        {label}
      </div>
    </div>
  )
}
