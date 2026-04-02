import { DeadLetterStatus } from '@/rpc/admin/deadletter/v1/deadletter_pb'

type StatusBadgeVariant = 'secondary' | 'default' | 'success' | 'warning' | 'destructive' | 'outline'

const STATUS_CONFIG: Record<number, { label: string; variant: StatusBadgeVariant }> = {
  [DeadLetterStatus.PENDING]: { label: 'Pending', variant: 'destructive' },
  [DeadLetterStatus.REQUEUED]: { label: 'Requeued', variant: 'warning' },
  [DeadLetterStatus.DISCARDED]: { label: 'Discarded', variant: 'secondary' },
}

export function getStatusConfig(status: DeadLetterStatus) {
  return STATUS_CONFIG[status] ?? { label: String(status), variant: 'secondary' as StatusBadgeVariant }
}

export const QUEUE_OPTIONS = [
  'outbox_event',
  'invoice_pdf_request',
  'credit_note_pdf_request',
  'webhook_out',
  'hubspot_sync',
  'pennylane_sync',
  'invoice_orchestration',
  'payment_request',
  'send_email_request',
  'quote_conversion',
  'bi_aggregation',
] as const
