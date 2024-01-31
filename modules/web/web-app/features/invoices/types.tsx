import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

export interface InvoicesSearch {
  text?: string
  status?: InvoiceStatus
}
