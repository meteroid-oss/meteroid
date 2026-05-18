import { Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { Merge } from 'lucide-react'
import { FC } from 'react'

/**
 * Small "inherited" indicator. Use it on rows where
 * the resolved value did not originate at the current entity level. The caller is responsible
 * for the tooltip text (see `buildInheritanceTooltip`).
 */
export const InheritedIcon: FC<{ tooltip: string }> = ({ tooltip }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <span className="text-muted-foreground cursor-help shrink-0" aria-label="Inherited">
        <Merge size={14}/>
      </span>
    </TooltipTrigger>
    <TooltipContent>{tooltip}</TooltipContent>
  </Tooltip>
)
