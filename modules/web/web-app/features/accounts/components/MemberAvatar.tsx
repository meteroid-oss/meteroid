import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger, cn } from '@md/ui'

import { useMembers } from '@/features/accounts/hooks/useMembers'

/** Initials of a member, with the name beside it or in a tooltip; "—" when nobody. */
export const MemberAvatar = ({
  userId,
  showName = false,
  short = false,
  size = 'md',
  className,
}: {
  userId?: string
  showName?: boolean
  /** Beside the circle, the first name rather than the full name. */
  short?: boolean
  size?: 'sm' | 'md'
  className?: string
}) => {
  const { byId, meId } = useMembers()

  if (!userId) {
    return (
      <span className={cn('text-muted-foreground', className)} aria-label="Nobody">
        —
      </span>
    )
  }

  const member = byId.get(userId)
  const name = member ? `${member.name}${userId === meId ? ' (you)' : ''}` : 'Former member'
  const shown = short && member ? member.shortName : name
  const circle = (
    <span
      aria-hidden
      className={cn(
        'inline-flex shrink-0 items-center justify-center rounded-full border border-border bg-muted font-medium text-foreground',
        size === 'sm' ? 'h-5 w-5 text-[9.5px]' : 'h-6 w-6 text-xs'
      )}
    >
      {member?.initials ?? '?'}
    </span>
  )

  if (showName) {
    return (
      <span className={cn('inline-flex min-w-0 items-center gap-2', className)}>
        {circle}
        <span
          className="truncate text-sm text-foreground"
          title={shown !== name ? name : undefined}
        >
          {shown}
        </span>
      </span>
    )
  }

  return (
    <TooltipProvider delayDuration={100}>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className={cn('inline-flex', className)} aria-label={name} role="img">
            {circle}
          </span>
        </TooltipTrigger>
        <TooltipContent>{name}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}
