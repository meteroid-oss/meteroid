import { Button } from '@ui/components'
import { cn } from '@ui/lib'
import { CopyIcon } from 'lucide-react'
import { toast } from 'sonner'

import { copyToClipboard } from '@/lib/helpers'

export const LocalId = ({
  localId,
  buttonClassName,
  className,
}: {
  localId: string
  buttonClassName?: string
  className?: string
}) => {
  return (
    <Button
      type="button"
      variant="special"
      size="content"
      title="Copy to clipboard"
      hasIcon
      className={cn(
        'text-xs px-3 py-2 bg-secondary text-secondary-foreground border-none font-normal',
        buttonClassName
      )}
      onClick={ev => {
        ev?.stopPropagation()
        ev?.preventDefault()
        copyToClipboard(localId, () => toast.success('Copied to clipboard : ' + localId))
      }}
    >
      <span className={cn('mr-2 whitespace-nowrap overflow-hidden text-ellipsis', className)}>
        {localId}
      </span>

      <CopyIcon size="10" />
    </Button>
  )
}
