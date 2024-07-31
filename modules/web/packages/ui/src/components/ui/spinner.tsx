import { Loader2 } from 'lucide-react'

import { cn } from '@ui/lib'

export const Spinner = ({ className, size }: { className?: string; size?: string | number }) => {
  return <Loader2 size={size} className={cn('animate-spin', className)} />
}
