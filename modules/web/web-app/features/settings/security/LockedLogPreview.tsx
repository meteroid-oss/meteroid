import { Badge, Card } from '@md/ui'
import {
  FileText,
  LogIn,
  Mail,
  Pencil,
  PlusCircle,
  ShieldAlert,
  ShieldCheck,
  Trash2,
} from 'lucide-react'
import { ReactNode } from 'react'

import { LockedFeatureCard } from '@/components/LockedFeatureCard'

const LABEL = { auditLog: 'The audit log', securityLog: 'The security log' }

// Renders a blurred, non-interactive sample of a gated feature behind the
// standard lock card, so viewers can see what they'd unlock.
export const LockedPreview = ({
  feature,
  title,
  children,
}: {
  feature: 'auditLog' | 'securityLog'
  title: string
  children: ReactNode
}) => (
  <div className="relative overflow-hidden rounded-lg">
    <div aria-hidden className="pointer-events-none select-none opacity-70 blur-[2.5px]">
      {children}
    </div>
    <div className="absolute inset-0 flex items-center justify-center bg-background/55">
      <LockedFeatureCard label={LABEL[feature]} title={title} />
    </div>
  </div>
)

const actor = (s: string) => <span className="font-medium text-foreground">{s}</span>

const FakeToolbar = ({ widths }: { widths: number[] }) => (
  <div className="mb-3 flex gap-2">
    {widths.map((w, i) => (
      <div
        key={i}
        className="h-9 rounded-md border border-border bg-input"
        style={{ width: w }}
      />
    ))}
  </div>
)

const SECURITY_ROWS: { icon: ReactNode; title: ReactNode; meta: string; when: string }[] = [
  {
    icon: <LogIn className="h-4 w-4 text-success" />,
    title: <>{actor('gaspard@acme.com')} signed in</>,
    meta: '89.44.12.3 · Chrome on macOS',
    when: '2m ago',
  },
  {
    icon: <ShieldCheck className="h-4 w-4 text-success" />,
    title: <>{actor('maya@acme.com')} passed the MFA challenge</>,
    meta: 'Firefox on Windows',
    when: '1h ago',
  },
  {
    icon: <ShieldAlert className="h-4 w-4 text-destructive" />,
    title: <>Failed sign-in for {actor('ravi@acme.com')} — wrong password</>,
    meta: '2.44.9.10 · Safari on iOS',
    when: '3h ago',
  },
  {
    icon: <Mail className="h-4 w-4 text-muted-foreground" />,
    title: (
      <>
        {actor('gaspard@acme.com')} invited {actor('sam@acme.com')}
      </>
    ),
    meta: 'Owner → Member',
    when: 'yesterday',
  },
  {
    icon: <ShieldCheck className="h-4 w-4 text-success" />,
    title: <>{actor('maya@acme.com')} enabled two-factor authentication</>,
    meta: 'Chrome on macOS',
    when: '2 days ago',
  },
]

export const SecurityLogSample = () => (
  <div>
    <FakeToolbar widths={[160, 200, 130]} />
    <Card className="px-2 py-2">
      <div className="divide-y divide-border">
        {SECURITY_ROWS.map((r, i) => (
          <div key={i} className="grid grid-cols-[24px_1fr_auto] items-start gap-3 px-3 py-3">
            <div className="pt-0.5">{r.icon}</div>
            <div className="space-y-1">
              <div className="text-sm">{r.title}</div>
              <div className="text-xs text-muted-foreground">{r.meta}</div>
            </div>
            <div className="whitespace-nowrap text-xs text-muted-foreground">{r.when}</div>
          </div>
        ))}
      </div>
    </Card>
  </div>
)

const AUDIT_ROWS: { icon: ReactNode; actor: string; action: ReactNode; ref: string; when: string }[] =
  [
    {
      icon: <PlusCircle className="h-4 w-4 text-success" />,
      actor: 'gaspard@acme.com',
      action: 'created invoice',
      ref: 'INV-1042',
      when: '5m ago',
    },
    {
      icon: <Pencil className="h-4 w-4 text-muted-foreground" />,
      actor: 'maya@acme.com',
      action: 'updated subscription',
      ref: 'Globex Inc.',
      when: '32m ago',
    },
    {
      icon: <FileText className="h-4 w-4 text-muted-foreground" />,
      actor: 'System',
      action: 'finalized invoice',
      ref: 'INV-1041',
      when: '1h ago',
    },
    {
      icon: <Trash2 className="h-4 w-4 text-destructive" />,
      actor: 'ravi@acme.com',
      action: 'deleted coupon',
      ref: 'SUMMER10',
      when: '4h ago',
    },
    {
      icon: <Pencil className="h-4 w-4 text-muted-foreground" />,
      actor: 'gaspard@acme.com',
      action: 'updated plan',
      ref: 'Pro',
      when: 'yesterday',
    },
  ]

export const AuditLogSample = () => (
  <div>
    <FakeToolbar widths={[180, 160, 130]} />
    <Card className="px-2 py-2">
      <div className="divide-y divide-border">
        {AUDIT_ROWS.map((r, i) => (
          <div key={i} className="flex items-center gap-3 px-3 py-3">
            <div className="shrink-0">{r.icon}</div>
            <div className="text-sm">
              {actor(r.actor)} {r.action}{' '}
              <Badge variant="outline" size="sm" className="font-normal">
                {r.ref}
              </Badge>
            </div>
            <div className="ml-auto whitespace-nowrap text-xs text-muted-foreground">{r.when}</div>
          </div>
        ))}
      </div>
    </Card>
  </div>
)
