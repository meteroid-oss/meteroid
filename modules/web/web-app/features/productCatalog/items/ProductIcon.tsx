import { cn } from '@md/ui'
import {
  BarChart3Icon,
  BotIcon,
  BoxIcon,
  BrainIcon,
  CloudIcon,
  CodeIcon,
  CpuIcon,
  DatabaseIcon,
  GaugeIcon,
  GlobeIcon,
  HardDriveIcon,
  HeadphonesIcon,
  KeyIcon,
  LayersIcon,
  LifeBuoyIcon,
  LineChartIcon,
  LockIcon,
  MailIcon,
  MessageSquareIcon,
  NetworkIcon,
  PackageIcon,
  PhoneIcon,
  PuzzleIcon,
  RocketIcon,
  ServerIcon,
  ShieldIcon,
  SparklesIcon,
  TerminalIcon,
  UserIcon,
  UsersIcon,
  WrenchIcon,
  ZapIcon,
} from 'lucide-react'

import type { LucideIcon } from 'lucide-react'

// The server accepts these names and tokens only (meteroid-store domain/products.rs).
export const PRODUCT_ICONS: Record<string, LucideIcon> = {
  box: BoxIcon,
  package: PackageIcon,
  cpu: CpuIcon,
  server: ServerIcon,
  database: DatabaseIcon,
  cloud: CloudIcon,
  zap: ZapIcon,
  bot: BotIcon,
  sparkles: SparklesIcon,
  brain: BrainIcon,
  'message-square': MessageSquareIcon,
  mail: MailIcon,
  phone: PhoneIcon,
  users: UsersIcon,
  user: UserIcon,
  shield: ShieldIcon,
  lock: LockIcon,
  key: KeyIcon,
  globe: GlobeIcon,
  'bar-chart-3': BarChart3Icon,
  'line-chart': LineChartIcon,
  gauge: GaugeIcon,
  'hard-drive': HardDriveIcon,
  network: NetworkIcon,
  code: CodeIcon,
  terminal: TerminalIcon,
  layers: LayersIcon,
  puzzle: PuzzleIcon,
  rocket: RocketIcon,
  headphones: HeadphonesIcon,
  'life-buoy': LifeBuoyIcon,
  wrench: WrenchIcon,
}

/** Each color's tile (light tint, colored icon) and its swatch in the picker. */
export const PRODUCT_COLORS: Record<string, { tile: string; dot: string }> = {
  slate: {
    tile: 'bg-slate-500/10 text-slate-600 dark:bg-slate-400/15 dark:text-slate-300',
    dot: 'bg-slate-500',
  },
  blue: {
    tile: 'bg-blue-500/10 text-blue-600 dark:bg-blue-400/15 dark:text-blue-400',
    dot: 'bg-blue-500',
  },
  green: {
    tile: 'bg-green-500/10 text-green-600 dark:bg-green-400/15 dark:text-green-400',
    dot: 'bg-green-500',
  },
  amber: {
    tile: 'bg-amber-500/10 text-amber-600 dark:bg-amber-400/15 dark:text-amber-400',
    dot: 'bg-amber-500',
  },
  red: {
    tile: 'bg-red-500/10 text-red-600 dark:bg-red-400/15 dark:text-red-400',
    dot: 'bg-red-500',
  },
  violet: {
    tile: 'bg-violet-500/10 text-violet-600 dark:bg-violet-400/15 dark:text-violet-400',
    dot: 'bg-violet-500',
  },
  pink: {
    tile: 'bg-pink-500/10 text-pink-600 dark:bg-pink-400/15 dark:text-pink-400',
    dot: 'bg-pink-500',
  },
  teal: {
    tile: 'bg-teal-500/10 text-teal-600 dark:bg-teal-400/15 dark:text-teal-400',
    dot: 'bg-teal-500',
  },
}

const NEUTRAL = 'border border-border bg-background text-muted-foreground'

const SIZES = {
  sm: { tile: 'size-5 rounded', icon: 12 },
  md: { tile: 'size-7 rounded-md', icon: 14 },
}

export type ProductAppearance = { icon?: string; color?: string }

/** A product's icon in its tinted square: the box, neutral, until the tenant picks one. */
export const ProductIcon = ({
  icon,
  color,
  size = 'md',
  className,
}: ProductAppearance & { size?: keyof typeof SIZES; className?: string }) => {
  const Icon = (icon && PRODUCT_ICONS[icon]) || BoxIcon
  const tint = (color && PRODUCT_COLORS[color]?.tile) || NEUTRAL
  return (
    <span
      aria-hidden
      className={cn(
        'inline-flex shrink-0 items-center justify-center',
        SIZES[size].tile,
        tint,
        className
      )}
    >
      <Icon size={SIZES[size].icon} />
    </span>
  )
}
