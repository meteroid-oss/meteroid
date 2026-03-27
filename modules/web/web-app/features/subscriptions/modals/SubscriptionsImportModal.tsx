import { FileSpreadsheetIcon } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { z } from 'zod'

import { CSVImportDialog } from '@/components/CSVImportDialog'
import { useBatchJobCreate, CreateBatchJobRequest } from '@/features/batch-jobs/useBatchJobCreate'
import { BatchJobType } from '@/rpc/api/batchjobs/v1/models_pb'

const DATE_RE = /^\d{4}-\d{2}-\d{2}$/
const BOOL_RE = /^(true|false)$/

const optionalValid = (re: RegExp, msg: string) =>
  z.string().refine(v => !v || re.test(v), msg).optional()

const subscriptionRowSchema = z.object({
  customer_id_or_alias: z.string().min(1, 'Required'),
  plan_id: z.string().min(1, 'Required'),
  start_date: z.string().regex(DATE_RE, 'Must be YYYY-MM-DD'),
  activation_condition: z.enum(['ON_START_DATE', 'ON_CHECKOUT', 'MANUAL'], {
    message: 'Must be ON_START_DATE, ON_CHECKOUT, or MANUAL',
  }),
  auto_advance_invoices: z.string().regex(BOOL_RE, 'Must be true or false'),
  charge_automatically: z.string().regex(BOOL_RE, 'Must be true or false'),
  skip_past_invoices: z.string().regex(BOOL_RE, 'Must be true or false'),
  end_date: optionalValid(DATE_RE, 'Must be YYYY-MM-DD'),
  billing_day_anchor: z.string().refine(v => {
    if (!v) return true
    const n = Number(v)
    return Number.isInteger(n) && n >= 1 && n <= 31
  }, 'Must be 1–31').optional(),
  net_terms: optionalValid(/^\d+$/, 'Must be a non-negative integer'),
  payment_method: optionalValid(/^(ONLINE|BANK_TRANSFER|EXTERNAL)$/, 'Must be ONLINE, BANK_TRANSFER, or EXTERNAL'),
})

interface SubscriptionsImportModalProps {
  openState: [boolean, (open: boolean) => void]
  onSuccess?: () => void
}

export const SubscriptionsImportModal: FunctionComponent<SubscriptionsImportModalProps> = ({
  openState,
}) => {
  const [isOpen, setIsOpen] = openState
  const [uploadFile, setUploadFile] = useState<File | null>(null)
  const [csvOptions, setCsvOptions] = useState({
    delimiter: ',',
  })
  const [batchJobId, setBatchJobId] = useState<string | null>(null)
  const [duplicateDetected, setDuplicateDetected] = useState(false)

  const createMutation = useBatchJobCreate({
    onSuccess: (jobId) => setBatchJobId(jobId),
    onDuplicate: () => setDuplicateDetected(true),
  })

  const buildRequest = (forceDuplicate = false) => {
    if (!uploadFile) return null
    return uploadFile.arrayBuffer().then(
      buffer =>
        new CreateBatchJobRequest({
          fileData: new Uint8Array(buffer),
          jobType: BatchJobType.SUBSCRIPTION_CSV_IMPORT,
          fileName: uploadFile.name,
          forceDuplicate,
          params: {
            delimiter: csvOptions.delimiter,
          },
        })
    )
  }

  const handleFileUpload = async () => {
    const request = await buildRequest()
    if (request) createMutation.mutate(request)
  }

  const handleForceUpload = async () => {
    const request = await buildRequest(true)
    if (request) createMutation.mutate(request)
  }

  const handleCloseModal = () => {
    setIsOpen(false)
    setUploadFile(null)
    setBatchJobId(null)
    setDuplicateDetected(false)
  }

  return (
    <CSVImportDialog
      isOpen={isOpen}
      setIsOpen={setIsOpen}
      uploadFile={uploadFile}
      setUploadFile={setUploadFile}
      csvOptions={csvOptions}
      setCsvOptions={setCsvOptions}
      isUploading={createMutation.isPending}
      batchJobId={batchJobId}
      onUpload={handleFileUpload}
      onForceUpload={handleForceUpload}
      onClose={handleCloseModal}
      duplicateDetected={duplicateDetected}
      entityName="subscriptions"
      dialogTitle="Import Subscriptions from CSV"
      dialogDescription="Import subscription data from a CSV file. All imports must include headers."
      dialogIcon={<FileSpreadsheetIcon className="h-5 w-5" />}
      rowSchema={subscriptionRowSchema}
      showRejectOnError={false}
      requiredColumns={[
        { name: 'customer_id_or_alias', tooltipMessage: 'Customer ID or alias' },
        { name: 'plan_id', tooltipMessage: 'Plan Handle' },
        { name: 'start_date', tooltipMessage: 'YYYY-MM-DD format' },
        {
          name: 'activation_condition',
          tooltipMessage: 'ON_START_DATE, ON_CHECKOUT, or MANUAL',
        },
        { name: 'auto_advance_invoices', tooltipMessage: 'true or false' },
        { name: 'charge_automatically', tooltipMessage: 'true or false' },
        { name: 'skip_past_invoices', tooltipMessage: 'true or false' },
      ]}
      optionalColumns={[
        {
          name: 'idempotency_key',
          tooltipMessage: 'Unique key to prevent duplicate imports on retry',
        },
        { name: 'plan_version', tooltipMessage: 'Version number — defaults to latest published' },
        { name: 'billing_day_anchor', tooltipMessage: 'Day of month (1–31)' },
        { name: 'end_date', tooltipMessage: 'YYYY-MM-DD format' },
        { name: 'net_terms', tooltipMessage: 'Days until payment due' },
        {
          name: 'payment_method',
          tooltipMessage: 'ONLINE, BANK_TRANSFER, or EXTERNAL',
        },
        { name: 'purchase_order' },
      ]}
      additionalInfo={
        <ul>
          <li>• Headers are required in the first row</li>
        </ul>
      }
    />
  )
}
