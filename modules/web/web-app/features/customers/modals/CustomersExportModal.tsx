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

interface CustomersExportModalProps {
  openState: [boolean, React.Dispatch<React.SetStateAction<boolean>>]
}

export const CustomersExportModal = ({
  openState: [visible, setVisible],
}: CustomersExportModalProps) => {
  const handleDownload = () => {
    console.log('downloaded')
    setVisible(false)
    toast.success('Export downloaded successfully')
  }

  return (
    <Dialog open={visible} onOpenChange={setVisible}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Export all customers</DialogTitle>
          <DialogDescription>
            Download an export of all your customers as .CSV file.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter className="mt-3">
          <Button size="sm" variant="secondary" onClick={() => setVisible(false)}>
            Cancel
          </Button>
          <Button size="sm" variant="default" onClick={handleDownload}>
            Download
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
