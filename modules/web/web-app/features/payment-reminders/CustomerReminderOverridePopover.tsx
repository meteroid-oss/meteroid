import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Pencil } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { CustomerReminderPreference } from '@/rpc/api/paymentreminders/v1/models_pb'
import {
  getCustomerReminderPreference,
  setCustomerReminderPreference,
} from '@/rpc/api/paymentreminders/v1/paymentreminders-PaymentRemindersService_connectquery'

type Mode = 'default' | 'sequence' | 'none'

const modeOf = (preference: CustomerReminderPreference): Mode =>
  preference.optOut ? 'none' : preference.sequenceId ? 'sequence' : 'default'

export const reminderPreferenceLabel = (preference: CustomerReminderPreference): string => {
  switch (modeOf(preference)) {
    case 'none':
      return 'None'
    case 'sequence': {
      const name = preference.availableSequences.find(s => s.id === preference.sequenceId)?.name
      return `Sequence: ${name ?? preference.sequenceId}`
    }
    default:
      return 'Default rules'
  }
}

interface Props {
  customerId: string
  preference: CustomerReminderPreference
}

export const CustomerReminderOverridePopover = ({ customerId, preference }: Props) => {
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const [mode, setMode] = useState<Mode>('default')
  const [sequenceId, setSequenceId] = useState('')

  const selectable = preference.availableSequences.filter(
    s => !s.archived || s.id === preference.sequenceId
  )

  const onOpenChange = (next: boolean) => {
    if (next) {
      setMode(modeOf(preference))
      setSequenceId(preference.sequenceId ?? selectable[0]?.id ?? '')
    }
    setOpen(next)
  }

  const saveMut = useMutation(setCustomerReminderPreference, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getCustomerReminderPreference,
          input: { customerId },
          cardinality: 'finite',
        }),
      })
      toast.success('Payment reminders updated')
      setOpen(false)
    },
    onError: err => toast.error(err.rawMessage || 'Could not update payment reminders'),
  })

  const save = () =>
    saveMut.mutate({
      customerId,
      optOut: mode === 'none',
      sequenceId: mode === 'sequence' ? sequenceId : undefined,
    })

  return (
    <Gate perm="customers:manage">
      <Popover open={open} onOpenChange={onOpenChange}>
        <PopoverTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            className="h-5 w-5 text-muted-foreground"
            aria-label="Edit payment reminders"
          >
            <Pencil size={12} />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="end" className="w-80 p-4">
          <div className="text-sm font-medium mb-3">Payment reminders</div>
          <RadioGroup value={mode} onValueChange={v => setMode(v as Mode)} className="gap-3">
            <div className="flex items-start space-x-2">
              <RadioGroupItem value="default" id="reminders-default" className="mt-0.5" />
              <Label htmlFor="reminders-default" className="text-sm font-normal cursor-pointer">
                Default
                <span className="block text-xs text-muted-foreground">
                  Audience rules pick the sequence for each invoice.
                </span>
              </Label>
            </div>
            <div className="flex items-start space-x-2">
              <RadioGroupItem
                value="sequence"
                id="reminders-sequence"
                className="mt-0.5"
                disabled={selectable.length === 0}
              />
              <Label
                htmlFor="reminders-sequence"
                className={`text-sm font-normal ${selectable.length === 0 ? 'opacity-50' : 'cursor-pointer'}`}
              >
                Specific sequence
                <span className="block text-xs text-muted-foreground">
                  {selectable.length === 0
                    ? 'No sequence available for this customer’s invoicing entity.'
                    : 'Always use this sequence, whatever the audiences say.'}
                </span>
              </Label>
            </div>
            {mode === 'sequence' && (
              <Select value={sequenceId} onValueChange={setSequenceId}>
                <SelectTrigger className="ml-6 w-[calc(100%-1.5rem)]">
                  <SelectValue placeholder="Pick a sequence" />
                </SelectTrigger>
                <SelectContent>
                  {selectable.map(s => (
                    <SelectItem key={s.id} value={s.id}>
                      {s.name}
                      {s.archived ? ' (archived)' : !s.enabled ? ' (disabled)' : ''}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
            <div className="flex items-start space-x-2">
              <RadioGroupItem value="none" id="reminders-none" className="mt-0.5" />
              <Label htmlFor="reminders-none" className="text-sm font-normal cursor-pointer">
                No reminders
                <span className="block text-xs text-muted-foreground">
                  This customer never receives payment reminders.
                </span>
              </Label>
            </div>
          </RadioGroup>
          <div className="mt-3 text-xs text-muted-foreground">
            No reminders also stops invoices already in progress; the other options apply to new
            invoices only.
          </div>
          <div className="mt-3 flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              size="sm"
              disabled={saveMut.isPending || (mode === 'sequence' && !sequenceId)}
              onClick={save}
            >
              Save
            </Button>
          </div>
        </PopoverContent>
      </Popover>
    </Gate>
  )
}
