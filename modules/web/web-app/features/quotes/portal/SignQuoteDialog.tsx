import { useMutation } from '@connectrpc/connect-query'
import { zodResolver } from '@hookform/resolvers/zod'
import {
  Button,
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
  cn,
} from '@md/ui'
import { Check, Hourglass, MailCheck, Pen } from 'lucide-react'
import { CSSProperties, useEffect, useRef, useState } from 'react'
import { useForm } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { slotItems } from '@/edition'
import { QUOTE_CHANGED_MESSAGE, rpcErrorMessage } from '@/features/quotes/detail/signers'
import { ConsentCheckbox } from '@/features/quotes/portal/ConsentCheckbox'
import {
  SIGNATURE_METHOD,
  SignaturePad,
  renderTypedSignature,
  scriptFontCovers,
} from '@/features/quotes/portal/SignaturePad'
import {
  setQuotePurchaseOrder,
  signQuote,
} from '@/rpc/portal/quotes/v1/quotes-PortalQuoteService_connectquery'
import { SignQuoteResponse } from '@/rpc/portal/quotes/v1/quotes_pb'

const signSchema = z
  .object({
    signedByName: z.string().trim().min(1, 'Enter your full name'),
    signedByTitle: z.string().trim(),
    mode: z.enum(['typed', 'drawn']),
    drawnSignature: z.string(),
    purchaseOrder: z.string().trim(),
    consent: z.boolean().refine(value => value, 'Accept the statement to sign'),
  })
  .refine(values => values.mode !== 'drawn' || values.drawnSignature.length > 0, {
    path: ['drawnSignature'],
    message: 'Draw your signature, or switch to Type',
  })

type SignForm = z.infer<typeof signSchema>

type Step = 'email' | 'sign' | 'done'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Seller branding tokens: the dialog renders outside of the portal root. */
  style?: CSSProperties
  quoteNumber?: string
  /** e.g. "€2,235.00 / month · 12 months" */
  summary?: string
  recipientEmail: string
  recipientName: string
  consentText: string
  documentHash?: string
  isCountersignature: boolean
  /** Verified signatures confirm the signer's email with a one-time code first. */
  verification: { required: boolean; verified: boolean; onVerified: () => void }
  /** The customer's purchase order, editable while signing. */
  purchaseOrder?: { enabled: boolean; current?: string }
  onPurchaseOrderSaved: () => void
  onSigned: (response: SignQuoteResponse) => void
  onStale: () => void
}

const StepIndicator = ({
  steps,
  current,
}: {
  steps: { id: Step; label: string }[]
  current: Step
}) => {
  const currentIndex = steps.findIndex(s => s.id === current)
  return (
    <ol aria-label="Signing steps" className="flex flex-wrap gap-x-5 gap-y-2 text-sm">
      {steps.map((step, index) => {
        const done = index < currentIndex || current === 'done'
        const active = index === currentIndex && current !== 'done'
        return (
          <li
            key={step.id}
            aria-current={active ? 'step' : undefined}
            className={cn(
              'flex items-center gap-2',
              active ? 'font-medium text-foreground' : 'text-muted-foreground'
            )}
          >
            <span
              aria-hidden
              className={cn(
                'flex h-6 w-6 items-center justify-center rounded-full text-xs font-semibold',
                done && 'bg-success/15 text-success',
                active && 'bg-foreground text-background',
                !done && !active && 'border border-border'
              )}
            >
              {done ? <Check className="h-3.5 w-3.5" /> : index + 1}
            </span>
            {step.label}
            {done && <span className="sr-only">(completed)</span>}
          </li>
        )
      })}
    </ol>
  )
}

export const SignQuoteDialog = ({
  open,
  onOpenChange,
  style,
  quoteNumber,
  summary,
  recipientEmail,
  recipientName,
  consentText,
  documentHash,
  isCountersignature,
  verification,
  purchaseOrder,
  onPurchaseOrderSaved,
  onSigned,
  onStale,
}: Props) => {
  const needsEmail = verification.required && !verification.verified
  const [step, setStep] = useState<Step>(needsEmail ? 'email' : 'sign')
  const [result, setResult] = useState<SignQuoteResponse>()
  const nameInput = useRef<HTMLInputElement | null>(null)
  const signMutation = useMutation(signQuote)
  const purchaseOrderMutation = useMutation(setQuotePurchaseOrder)
  const purchaseOrderEditable = Boolean(purchaseOrder?.enabled) && !isCountersignature

  const defaults: SignForm = {
    signedByName: recipientName,
    signedByTitle: '',
    mode: scriptFontCovers(recipientName) ? 'typed' : 'drawn',
    drawnSignature: '',
    purchaseOrder: purchaseOrder?.current ?? '',
    consent: false,
  }

  const form = useForm<z.input<typeof signSchema>, unknown, SignForm>({
    resolver: zodResolver(signSchema),
    defaultValues: defaults,
  })

  useEffect(() => {
    if (!open) return
    setStep(needsEmail ? 'email' : 'sign')
    setResult(undefined)
    form.reset(defaults)
    // Only on opening: later prop changes (a refetch) must not wipe what is being typed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  useEffect(() => {
    if (step === 'sign') nameInput.current?.focus()
  }, [step])

  const steps: { id: Step; label: string }[] = [
    ...(verification.required
      ? [{ id: 'email' as const, label: needsEmail ? 'Confirm email' : 'Email confirmed' }]
      : []),
    { id: 'sign', label: isCountersignature ? 'Your countersignature' : 'Your signature' },
    { id: 'done', label: 'Done' },
  ]

  const onSubmit = async (values: SignForm) => {
    if (purchaseOrderEditable && values.purchaseOrder !== (purchaseOrder?.current ?? '')) {
      try {
        await purchaseOrderMutation.mutateAsync({
          purchaseOrder: values.purchaseOrder || undefined,
        })
        onPurchaseOrderSaved()
      } catch (error) {
        toast.error(rpcErrorMessage(error, 'The purchase order could not be saved'))
        return
      }
    }

    const signatureData =
      values.mode === 'typed'
        ? await renderTypedSignature(values.signedByName)
        : values.drawnSignature
    if (!signatureData) {
      toast.error('The signature could not be captured')
      return
    }

    try {
      const response = await signMutation.mutateAsync({
        recipientEmail,
        signedByName: values.signedByName,
        signedByTitle: values.signedByTitle || undefined,
        signatureData,
        signatureMethod: SIGNATURE_METHOD[values.mode],
        documentHash,
        consentText,
      })
      setResult(response)
      setStep('done')
      onSigned(response)
    } catch (error) {
      const message = rpcErrorMessage(error, 'The quote could not be signed')
      if (message.startsWith(QUOTE_CHANGED_MESSAGE)) {
        onOpenChange(false)
        onStale()
      } else {
        toast.error(message)
      }
    }
  }

  const submitting = form.formState.isSubmitting
  const consented = form.watch('consent')
  const SignatureCode = slotItems('portal.quoteSignatureCode')[0]
  const mode = form.watch('mode')
  const name = form.watch('signedByName')

  return (
    <Dialog open={open} onOpenChange={next => !submitting && onOpenChange(next)}>
      <DialogContent className="max-w-xl" style={style}>
        <DialogHeader>
          <DialogTitle>
            {isCountersignature ? 'Countersign the quote' : 'Sign the quote'}
          </DialogTitle>
          {(quoteNumber || summary) && (
            <DialogDescription>
              {[quoteNumber, summary].filter(Boolean).join(' · ')}
            </DialogDescription>
          )}
        </DialogHeader>

        <StepIndicator steps={steps} current={step} />

        {step === 'email' && (
          <>
            {SignatureCode ? (
              <SignatureCode
                email={recipientEmail}
                onVerified={() => {
                  verification.onVerified()
                  setStep('sign')
                }}
              />
            ) : (
              <p role="alert" className="text-sm text-destructive">
                Verified signing unavailable
              </p>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
            </DialogFooter>
          </>
        )}

        {step === 'sign' && (
          <Form {...form}>
            <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-5" noValidate>
              <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="signedByName"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Full name *</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          ref={element => {
                            field.ref(element)
                            nameInput.current = element
                          }}
                          autoComplete="name"
                          placeholder="Your full name"
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="signedByTitle"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Title (optional)</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          autoComplete="organization-title"
                          placeholder="e.g. VP Operations"
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>

              <SignaturePad
                name={name}
                mode={mode}
                onModeChange={value => form.setValue('mode', value)}
                onDrawnChange={value =>
                  form.setValue('drawnSignature', value, {
                    shouldValidate: form.formState.isSubmitted,
                  })
                }
                error={form.formState.errors.drawnSignature?.message}
              />

              {purchaseOrderEditable && (
                <FormField
                  control={form.control}
                  name="purchaseOrder"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Purchase order number (optional)</FormLabel>
                      <FormControl>
                        <Input {...field} placeholder="e.g. PO-88412" />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              )}

              <FormField
                control={form.control}
                name="consent"
                render={({ field }) => (
                  <FormItem>
                    <ConsentCheckbox
                      text={consentText}
                      checked={field.value}
                      onCheckedChange={field.onChange}
                    />
                    <FormMessage />
                  </FormItem>
                )}
              />

              <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => onOpenChange(false)}
                  disabled={submitting}
                >
                  Cancel
                </Button>
                <Button type="submit" variant="brand" hasIcon disabled={!consented || submitting}>
                  <Pen className="h-4 w-4" />
                  {submitting
                    ? 'Signing...'
                    : isCountersignature
                      ? 'Countersign'
                      : 'Sign and accept'}
                </Button>
              </div>
            </form>
          </Form>
        )}

        {step === 'done' && (
          <div className="space-y-5">
            <div role="status" className="flex flex-col items-center gap-3 py-4 text-center">
              {result?.accepted || !result?.awaitingCountersignature ? (
                <MailCheck aria-hidden className="h-10 w-10 text-success" />
              ) : (
                <Hourglass aria-hidden className="h-10 w-10 text-muted-foreground" />
              )}
              <p className="text-base font-semibold text-foreground">
                {result?.accepted
                  ? 'Signed and accepted'
                  : result?.awaitingCountersignature
                    ? 'Signed · awaiting countersignature'
                    : 'Signed'}
              </p>
            </div>
            <DialogFooter>
              <Button type="button" variant="brand" autoFocus onClick={() => onOpenChange(false)}>
                Close
              </Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
