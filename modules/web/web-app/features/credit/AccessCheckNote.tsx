import { cn, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { InfoIcon } from 'lucide-react'

export const ACCESS_CHECK_EXPLAINER =
  'Meteroid never rejects usage. This setting only changes what the access check returns: your product must call the check and refuse the request, and any usage that still arrives is recorded and billed.'

export const BALANCE_LAG_NOTE =
  'Balances and limits can take a few minutes to update, so some usage may be recorded and billed after a limit is reached.'

/** The info icon explaining what limits, run-out rules and hard budgets actually do. */
export const AccessCheckInfo = ({ className }: { className?: string }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <InfoIcon
        size={12}
        aria-label="How access checks work"
        className={cn('inline shrink-0 cursor-help text-muted-foreground', className)}
      />
    </TooltipTrigger>
    <TooltipContent className="max-w-72">{ACCESS_CHECK_EXPLAINER}</TooltipContent>
  </Tooltip>
)

/** Shown once per screen where balances or limits are shown or set. */
export const BalanceLagNote = ({ className }: { className?: string }) => (
  <p className={cn('text-xs text-muted-foreground', className)}>{BALANCE_LAG_NOTE}</p>
)
