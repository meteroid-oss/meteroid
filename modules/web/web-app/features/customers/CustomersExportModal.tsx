import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
} from '@ui/components'

interface CustomersExportModalProps {
  openState: [boolean, React.Dispatch<React.SetStateAction<boolean>>]
}

export const CustomersExportModal = ({
  openState: [visible, setVisible],
}: CustomersExportModalProps) => (
  <Dialog open={visible} onOpenChange={setVisible}>
    <DialogContent>
      <DialogHeader>Export all customers</DialogHeader>
      <DialogDescription className="-mt-2">
        Download an export of all your customers as .CSV file.
      </DialogDescription>
      <DialogFooter className="mt-3">
        <Button size="sm" variant="secondary" onClick={() => setVisible(false)}>
          Cancel
        </Button>
        <Button size="sm" variant="default" onClick={() => console.log('downloaded')}>
          Download
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
)
