import { Button, ButtonVariants } from '@ui/components'
import { cn } from '@ui/lib'
import { CopyIcon } from 'lucide-react'
import { toast } from 'sonner'

import { copyToClipboard } from '@/lib/helpers'

export const CopyToClipboardButton = ({
  text,
  textToCopy = text,
  buttonClassName,
  className,
  buttonVariant,
}: {
  text: string
  textToCopy?: string
  buttonClassName?: string
  buttonVariant?: ButtonVariants['variant']
  className?: string
}) => {
  return (
    <Button
      type="button"
      variant={buttonVariant ?? 'special'}
      size="content"
      title="Copy to clipboard"
      hasIcon
      className={cn('text-xs px-3 py-2 border-none font-normal', buttonClassName)}
      onClick={ev => {
        ev?.stopPropagation()
        ev?.preventDefault()
        copyToClipboard(textToCopy ?? text, () => toast.success('Copied to clipboard!'))
      }}
    >
      <span className={cn('  whitespace-nowrap overflow-hidden text-ellipsis', className)}>
        {text}
      </span>

      <CopyIcon size="10" />
    </Button>
  )
}
