import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@ui/components'
import { toast } from 'sonner'

//This was not present in the designs but will be needed for the invoice creation

interface CustomerInvoiceModalProps {
  openState: [boolean, React.Dispatch<React.SetStateAction<boolean>>]
}

export const CustomerInvoiceModal = ({
  openState: [visible, setVisible],
}: CustomerInvoiceModalProps) => {
  const handleInvoice = () => {
    setVisible(false)
    toast.success('Invoice created successfully')
  }

  return (
    <Dialog open={visible} onOpenChange={setVisible}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create an invoice</DialogTitle>
          <DialogDescription>
            Create an invoice for your client. You can add items, discounts, and taxes to the
            invoice.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter className="mt-3">
          <Button size="sm" variant="secondary" onClick={() => setVisible(false)}>
            Cancel
          </Button>
          <Button size="sm" variant="default" onClick={handleInvoice}>
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
