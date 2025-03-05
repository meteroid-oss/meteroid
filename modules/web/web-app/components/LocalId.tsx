import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { cn } from '@ui/lib'

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
    <CopyToClipboardButton
      text={localId}
      buttonClassName={cn(' bg-secondary text-secondary-foreground', buttonClassName)}
      className={className}
    />
  )
}
