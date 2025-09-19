import { useMutation } from '@connectrpc/connect-query'
import { zodResolver } from '@hookform/resolvers/zod'
import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { AlertCircle, Check, Pen, X } from 'lucide-react'
import { FC, useRef, useState } from 'react'
import { useForm } from 'react-hook-form'
import SignatureCanvas from 'react-signature-canvas'
import { z } from 'zod'

import { QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import { QuotePortalDetails } from '@/rpc/portal/quotes/v1/models_pb'
import {
  getQuotePortal,
  signQuote,
} from '@/rpc/portal/quotes/v1/quotes-PortalQuoteService_connectquery'

import { QuoteView } from './QuoteView'

export interface QuotePortalViewProps {
  quoteData: QuotePortalDetails
}

const signQuoteSchema = z.object({
  recipientEmail: z.string().email('Valid email is required'),
  signedByName: z.string().min(1, 'Full name is required'),
  signedByTitle: z.string().optional(),
  signatureData: z.string().min(1, 'Signature is required'),
  signatureMethod: z.string().default('digital_signature'),
})

type SignQuoteFormData = z.infer<typeof signQuoteSchema>

const QuotePortalView: FC<QuotePortalViewProps> = ({ quoteData }) => {
  const [showSignDialog, setShowSignDialog] = useState(false)
  const signaturePadRef = useRef<SignatureCanvas>(null)

  const queryClient = useQueryClient()

  const signQuoteMutation = useMutation(signQuote, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: [getQuotePortal.service.typeName] })
    },
  })

  const quote = quoteData.quote
  const customer = quoteData.customer
  const signatures = quoteData.signatures || []

  const isExpired = quote?.expiresAt && new Date(quote.expiresAt) < new Date()
  const isDraft = quote?.status === QuoteStatus.DRAFT
  const canSign = quote?.status === QuoteStatus.PENDING
  const isAccepted = quote?.status === QuoteStatus.ACCEPTED
  const isDeclined = quote?.status === QuoteStatus.DECLINED

  const currentUserEmail = quoteData.currentRecipientEmail || customer?.billingEmail || ''
  const currentUserName = quoteData.currentRecipientName || customer?.name || ''
  const hasCurrentUserSigned = signatures.some(s => s.signedByEmail === currentUserEmail)

  const signForm = useForm<SignQuoteFormData>({
    resolver: zodResolver(signQuoteSchema),
    defaultValues: {
      recipientEmail: currentUserEmail,
      signedByName: currentUserName,
      signedByTitle: '',
      signatureData: '',
      signatureMethod: 'digital_signature',
    },
  })

  const clearSignature = () => {
    if (signaturePadRef.current) {
      signaturePadRef.current.clear()
      signForm.setValue('signatureData', '')
    }
  }

  const saveSignature = () => {
    if (signaturePadRef.current) {
      const dataURL = signaturePadRef.current.toDataURL()
      signForm.setValue('signatureData', dataURL)
    }
  }

  const onSignSubmit = async (formData: SignQuoteFormData) => {
    try {
      await signQuoteMutation.mutateAsync({
        recipientEmail: currentUserEmail,
        signedByName: formData.signedByName,
        signedByTitle: formData.signedByTitle || undefined,
        signatureData: formData.signatureData,
        signatureMethod: formData.signatureMethod,
      })
      setShowSignDialog(false)
    } catch (error) {
      console.error('Failed to sign quote:', error)
    }
  }

  const getStatusBadge = () => {
    if (quote?.status === undefined) return null

    switch (quote.status) {
      case QuoteStatus.DRAFT:
        return <Badge variant="secondary">Draft</Badge>
      case QuoteStatus.PENDING:
        return <Badge variant="warning">Pending Signature</Badge>
      case QuoteStatus.ACCEPTED:
        return <Badge variant="success">Accepted</Badge>
      case QuoteStatus.DECLINED:
        return <Badge variant="destructive">Declined</Badge>
      case QuoteStatus.EXPIRED:
        return <Badge variant="outline">Expired</Badge>
      case QuoteStatus.CANCELLED:
        return <Badge variant="outline">Cancelled</Badge>
      default:
        return <Badge variant="outline">{quote.status}</Badge>
    }
  }

  return (
    <div className="min-h-screen  ">
      {/* Header */}
      <div className="  border-b">
        <div className="max-w-4xl mx-auto px-6 py-4">
          <div className="flex justify-between items-center">
            <div>
              <h1 className="text-2xl font-semibold">Quote {quote?.quoteNumber}</h1>
            </div>
            <div className="text-right">
              {getStatusBadge()}
              {isExpired && quote?.expiresAt && (
                <p className="text-sm text-red-600 mt-1">
                  Expired on {new Date(quote.expiresAt).toLocaleDateString()}
                </p>
              )}
            </div>
          </div>
        </div>
      </div>

      <div className="max-w-4xl mx-auto px-6 py-8">
        <div className="space-y-6">
          {/* Status Banner */}
          {isDraft && (
            <Alert>
              <AlertCircle className="h-4 w-4" />
              <AlertDescription>
                This quote has not been published yet and cannot be signed.
              </AlertDescription>
            </Alert>
          )}

          {isExpired && (
            <Alert variant="destructive">
              <AlertCircle className="h-4 w-4" />
              <AlertDescription>
                This quote has expired and can no longer be signed.
              </AlertDescription>
            </Alert>
          )}

          {isAccepted && (
            <Alert>
              <Check className="h-4 w-4" />
              <AlertDescription>This quote has been signed.</AlertDescription>
            </Alert>
          )}

          {isDeclined && (
            <Alert variant="destructive">
              <X className="h-4 w-4" />
              <AlertDescription>This quote has been declined.</AlertDescription>
            </Alert>
          )}

          {/* Quote Document */}
          <Card>
            <CardContent className="p-8">
              {quote && (
                <QuoteView
                  quote={{
                    components: quoteData.components,
                    quote: quoteData.quote,
                    customer: quoteData.customer,
                    invoicingEntity: quoteData.entity,
                  }}
                  mode="portal"
                />
              )}
            </CardContent>
          </Card>

          {/* Signatures Section */}
          {signatures.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle>Signatures</CardTitle>
                <CardDescription>Digital signatures for this quote</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="space-y-4">
                  {signatures.map((signature, index) => (
                    <div
                      key={index}
                      className="flex justify-between items-center p-3 bg-muted rounded-lg"
                    >
                      <div>
                        <p className="font-medium">{signature.signedByName}</p>
                        <p className="text-sm text-muted-foreground">{signature.signedByEmail}</p>
                        {signature.signedByTitle && (
                          <p className="text-sm text-muted-foreground">{signature.signedByTitle}</p>
                        )}
                      </div>
                      <div className="text-right">
                        <p className="text-sm font-medium">
                          {new Date(signature.signedAt).toLocaleDateString()}
                        </p>
                        <p className="text-sm text-muted-foreground">{signature.signatureMethod}</p>
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>
          )}

          {/* Sign Quote Action */}
          {canSign && !isExpired && !hasCurrentUserSigned && (
            <Card>
              <CardHeader>
                <CardTitle>Sign Quote</CardTitle>
                <CardDescription>
                  Please review the quote above and provide your digital signature to accept
                </CardDescription>
              </CardHeader>
              <CardContent>
                <Button onClick={() => setShowSignDialog(true)} size="lg" className="w-full">
                  <Pen className="w-5 h-5 mr-2" />
                  Sign Quote
                </Button>
              </CardContent>
            </Card>
          )}

          {hasCurrentUserSigned && (
            <Card>
              <CardContent className="p-6 text-center">
                <Check className="h-12 w-12 text-green-500 mx-auto mb-4" />
                <h3 className="text-lg font-semibold mb-2">Quote Signed</h3>
                <p className="text-muted-foreground">You have successfully signed this quote.</p>
              </CardContent>
            </Card>
          )}
        </div>
      </div>

      {/* Sign Quote Dialog */}
      <Dialog open={showSignDialog} onOpenChange={setShowSignDialog}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>Sign Quote</DialogTitle>
            <DialogDescription>
              By signing this quote, you agree to the terms and pricing outlined above.
            </DialogDescription>
          </DialogHeader>

          <Form {...signForm}>
            <form onSubmit={signForm.handleSubmit(onSignSubmit)} className="space-y-6">
              <div className="grid grid-cols-2 gap-4">
                <FormField
                  control={signForm.control}
                  name="signedByName"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Full Name *</FormLabel>
                      <FormControl>
                        <Input {...field} placeholder="Your full name" />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                {/* <FormField
                  control={signForm.control}
                  name="recipientEmail"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Email *</FormLabel>
                      <FormControl>
                        <Input {...field} type="email" placeholder="your@email.com" />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                /> */}

                <FormField
                  control={signForm.control}
                  name="signedByTitle"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Title/Position (Optional)</FormLabel>
                      <FormControl>
                        <Input {...field} placeholder="e.g., CEO, Purchasing Manager" />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              {/* Signature Pad */}
              <div className="space-y-2">
                <FormLabel>Digital Signature *</FormLabel>
                <div className="border-2 border-dashed border-gray-300 rounded-lg p-4">
                  <SignatureCanvas
                    ref={signaturePadRef}
                    penColor="black"
                    throttle={8}
                    backgroundColor="white"
                    canvasProps={{
                      width: 600,
                      height: 200,
                      className: 'signature-canvas  ',
                    }}
                    onEnd={saveSignature}
                  />
                </div>
                <div className="flex justify-between items-center">
                  <p className="text-sm text-muted-foreground">Sign above to accept the quote</p>
                  <Button type="button" variant="outline" size="sm" onClick={clearSignature}>
                    Clear Signature
                  </Button>
                </div>
                {signForm.formState.errors.signatureData && (
                  <p className="text-sm text-red-600">
                    {signForm.formState.errors.signatureData.message}
                  </p>
                )}
              </div>

              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setShowSignDialog(false)}>
                  Cancel
                </Button>
                <Button type="submit" disabled={signQuoteMutation.isPending}>
                  <Pen className="w-4 h-4 mr-2" />
                  {signQuoteMutation.isPending ? 'Signing...' : 'Sign Quote'}
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </DialogContent>
      </Dialog>
    </div>
  )
}

export default QuotePortalView
