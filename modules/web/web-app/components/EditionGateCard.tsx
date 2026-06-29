import { Button, Card, CardContent } from '@md/ui'
import { ExternalLink } from 'lucide-react'

import type { FunctionComponent, ReactNode } from 'react'

interface EditionGateCardProps {
  /** Icon rendered in the muted circle (e.g. `<ShieldCheck size={24} strokeWidth={1.5} />`). */
  icon: ReactNode
  title: string
  description: ReactNode
  /**
   * `centered` fills the available space and centers the card — for full pages / settings tabs.
   * `inline` renders the bare card — for embedding inside an existing settings layout.
   */
  layout?: 'centered' | 'inline'
  compareHref?: string
  learnMoreHref?: string
}

/**
 * Upgrade prompt shown where a Meteroid Cloud / Enterprise feature would be in the
 * open-source edition (audit log, MFA, security, …). Keeps the gated surfaces visually
 * consistent and in one place.
 */
export const EditionGateCard: FunctionComponent<EditionGateCardProps> = ({
  icon,
  title,
  description,
  layout = 'centered',
  compareHref = 'https://meteroid.com/pricing',
  learnMoreHref = 'https://docs.meteroid.com',
}) => {
  const card = (
    <Card className={layout === 'centered' ? 'max-w-xl w-full border-dashed' : 'border-dashed'}>
      <CardContent className="flex flex-col items-center text-center gap-4 py-12 px-8">
        <div className="flex items-center justify-center h-12 w-12 rounded-full bg-muted text-muted-foreground">
          {icon}
        </div>
        <div className="space-y-3">
          <h3 className="text-lg font-semibold">{title}</h3>
          <p className="text-sm text-muted-foreground max-w-md">{description}</p>
        </div>
        <div className="flex items-center gap-3 pt-2">
          <Button asChild variant="default">
            <a href={compareHref} target="_blank" rel="noreferrer">
              {/* Wrap the label so the brand foreground colour wins over the global `a` colour. */}
              <span className="inline-flex items-center text-brand-foreground dark:text-primary-foreground">
                Compare editions
                <ExternalLink size={14} className="ml-1.5" />
              </span>
            </a>
          </Button>
          <Button asChild variant="outline">
            <a href={learnMoreHref} target="_blank" rel="noreferrer">
              Learn more
            </a>
          </Button>
        </div>
      </CardContent>
    </Card>
  )

  if (layout === 'inline') {
    return card
  }

  return <div className="flex items-center justify-center w-full py-16">{card}</div>
}
