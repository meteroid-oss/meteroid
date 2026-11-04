import { Button, Card, cn } from '@md/ui'
import { Sparkles } from 'lucide-react'
import { Link } from 'react-router-dom'

import { helpTopicHref } from '@/features/help/topics'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsCloud } from '@/hooks/useIsCloud'
import { usePermissions } from '@/hooks/usePermissions'

// Shown in place of a gated surface in a production tenant. Renders static
// content only — the report component (and its data fetch) never mounts. In
// non-production tenants an add-on feature is open instead, badged as an add-on.

/** The add-on a feature belongs to, and the help topic carrying its enablement request. */
export interface LockedFeatureAddon {
  name: string
  helpTopic: string
  requestLabel: string
  /** What happens after the request, in one line. */
  requestNext: string
}

export const LockedFeatureCard = ({
  label,
  title,
  description,
  compact = false,
  addon,
  preview,
}: {
  /** The feature's name, as the start of a sentence (e.g. "The audit log"). */
  label: string
  title?: string
  /** What the surface would show, in one or two sentences. */
  description?: string
  /**
   * For a card that sits in a page stack (between a header and other content) rather than
   * filling the page: no vertical centring padding, and pane-sized type.
   */
  compact?: boolean
  addon?: LockedFeatureAddon
  /** A screenshot of the surface, shown in place of the icon. */
  preview?: { src: string; alt: string }
}) => {
  const basePath = useBasePath()
  const selfHosted = useIsCloud() === false
  const canManageLicense = usePermissions().has('organization:manage')

  return (
    <div className={cn('flex items-center justify-center', !compact && 'h-full py-16')}>
      <Card
        className={cn(
          'w-full flex flex-col items-center text-center gap-3',
          preview ? 'max-w-2xl' : 'max-w-md',
          compact ? 'p-6' : 'p-8'
        )}
      >
        {preview ? (
          <figure className="mb-2 w-full overflow-hidden rounded-md border border-border">
            <img src={preview.src} alt={preview.alt} loading="lazy" className="h-auto w-full" />
          </figure>
        ) : (
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-brand/10 text-brand">
            <Sparkles className="h-5 w-5" />
          </div>
        )}
        <div className={cn('font-medium text-foreground', compact ? 'text-[15px]' : 'text-base')}>
          {title ?? label}
        </div>
        {description && (
          <p className={cn('text-foreground', compact ? 'text-[13px]' : 'text-sm')}>
            {description}
          </p>
        )}
        <p className={cn('text-muted-foreground', compact ? 'text-[13px]' : 'text-sm')}>
          {selfHosted ? (
            <>
              {label} is{' '}
              {addon && (
                <>
                  part of <span className="text-foreground">{addon.name}</span>,{' '}
                </>
              )}
              available with a Meteroid license.
              {!canManageLicense && ' Ask an organization admin to get one in Settings → License.'}
            </>
          ) : addon ? (
            <>
              {label} is part of the{' '}
              <span className="text-foreground">{addon.name}</span> add-on, which
              isn&apos;t on your plan. It stays open in your non-production tenants, so you can try
              it there before enabling it here.
            </>
          ) : (
            <>
              {label} is not included in your current plan. Contact your account
              manager to unlock it.
            </>
          )}
        </p>
        {selfHosted ? (
          canManageLicense && (
            <Button variant="secondary" size="sm" asChild className="mt-1">
              <Link to={`${basePath}/settings/license`}>Get a license</Link>
            </Button>
          )
        ) : addon && (
          <>
            <Button variant="secondary" size="sm" asChild className="mt-1">
              <Link to={helpTopicHref(basePath, addon.helpTopic)}>{addon.requestLabel}</Link>
            </Button>
            <p className="text-xs text-muted-foreground">{addon.requestNext}</p>
          </>
        )}
      </Card>
    </div>
  )
}
