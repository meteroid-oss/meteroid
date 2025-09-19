import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@md/ui'
import { LucideIcon } from 'lucide-react'

interface InvoiceConfirmationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: () => void
  icon: LucideIcon
  title: string
  description: string
  invoiceNumber?: string
  confirmText?: string
  cancelText?: string
}

export const InvoiceConfirmationDialog: React.FC<InvoiceConfirmationDialogProps> = ({
  open,
  onOpenChange,
  onConfirm,
  icon: Icon,
  title,
  description,
  invoiceNumber,
  confirmText = 'Confirm',
  cancelText = 'Cancel',
}) => {
  const displayTitle = invoiceNumber ? `${invoiceNumber}: ${title}` : title

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle className="flex items-center gap-2 text-md">
            <Icon className="w-6 h-6 text-red-600"/>
            <span>{displayTitle}</span>
          </AlertDialogTitle>
          <AlertDialogDescription>
            {description}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{cancelText}</AlertDialogCancel>
          <AlertDialogAction
            onClick={onConfirm}
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
          >
            {confirmText}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
