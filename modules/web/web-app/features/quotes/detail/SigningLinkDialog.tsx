import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
} from '@md/ui'
import { Copy } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  defaultEmail: string
  isPending: boolean
  generate: (email: string) => Promise<string | undefined>
}

export const SigningLinkDialog = ({
  open,
  onOpenChange,
  defaultEmail,
  isPending,
  generate,
}: Props) => {
  const [email, setEmail] = useState(defaultEmail)
  const [url, setUrl] = useState('')

  useEffect(() => {
    if (open) {
      setEmail(defaultEmail)
      setUrl('')
    }
  }, [open, defaultEmail])

  const copy = async (text: string) => {
    await navigator.clipboard.writeText(text)
    toast.success('Link copied', { id: 'copy' })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Personal signing link</DialogTitle>
          <DialogDescription>Anyone with this link can sign as this recipient.</DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div>
            <Label htmlFor="recipient-email">Recipient email</Label>
            <Input
              id="recipient-email"
              type="email"
              value={email}
              onChange={e => {
                setEmail(e.target.value)
                setUrl('')
              }}
              placeholder="Enter recipient email"
              className="mt-1"
            />
          </div>

          {url && (
            <div>
              <Label htmlFor="signing-link-url">Portal URL</Label>
              <div className="flex gap-2 mt-1">
                <Input id="signing-link-url" value={url} readOnly />
                <Button
                  size="sm"
                  variant="outline"
                  aria-label="Copy the link"
                  onClick={() => void copy(url)}
                >
                  <Copy className="w-4 h-4" />
                </Button>
              </div>
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Close
          </Button>
          <Button
            onClick={async () => {
              const generated = await generate(email.trim())
              if (generated) setUrl(generated)
            }}
            disabled={!email.trim() || isPending}
          >
            {isPending ? 'Generating...' : 'Generate link'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
