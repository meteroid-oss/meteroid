import { CreditNoteStatus } from '@/rpc/api/creditnotes/v1/models_pb'

export interface CreditNotesSearch {
  text?: string
  status?: CreditNoteStatus
}
