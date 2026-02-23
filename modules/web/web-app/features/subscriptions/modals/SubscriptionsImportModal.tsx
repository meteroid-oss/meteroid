import { useMutation } from '@connectrpc/connect-query'
import { FileSpreadsheetIcon } from 'lucide-react'
import { FunctionComponent, useState } from 'react'
import { toast } from 'sonner'

import { CSVImportDialog, CSVImportResult } from '@/components/CSVImportDialog'
import { ingestCsv } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsIngestService_connectquery'
import { FileData, IngestCsvRequest } from '@/rpc/api/subscriptions/v1/subscriptions_pb'

interface SubscriptionsImportModalProps {
  openState: [boolean, (open: boolean) => void]
  onSuccess?: () => void
}

export const SubscriptionsImportModal: FunctionComponent<SubscriptionsImportModalProps> = ({
  openState,
  onSuccess,
}) => {
  const [isOpen, setIsOpen] = openState
  const [uploadFile, setUploadFile] = useState<File | null>(null)
  const [csvOptions, setCsvOptions] = useState({
    delimiter: ',',
    failOnError: false,
  })
  const [importResult, setImportResult] = useState<CSVImportResult | null>(null)

  const uploadMutation = useMutation(ingestCsv, {
    onSuccess: async response => {
      const { totalRows, successfulRows, failures } = response

      setImportResult({
        totalRows,
        successful: successfulRows,
        failures:
          failures?.map(f => ({
            rowNumber: f.rowNumber,
            identifier: '',
            reason: f.reason,
          })) || [],
      })

      if (!failures || failures.length === 0) {
        setIsOpen(false)
        setUploadFile(null)
        setImportResult(null)
        onSuccess?.()
        toast.success(`Successfully imported ${successfulRows} subscriptions`)
      }
    },
    onError: error => {
      setImportResult({
        totalRows: 0,
        successful: 0,
        failures: [{ rowNumber: 0, identifier: '', reason: error.message }],
      })
    },
  })

  const handleFileUpload = async () => {
    if (!uploadFile) return

    setImportResult(null)
    const buffer = await uploadFile.arrayBuffer()
    const request = new IngestCsvRequest({
      file: new FileData({ data: new Uint8Array(buffer) }),
      delimiter: csvOptions.delimiter,
      failOnError: csvOptions.failOnError,
    })

    uploadMutation.mutate(request)
  }

  const handleCloseModal = () => {
    setIsOpen(false)
    setUploadFile(null)
    setImportResult(null)
  }

  return (
    <CSVImportDialog
      isOpen={isOpen}
      setIsOpen={setIsOpen}
      uploadFile={uploadFile}
      setUploadFile={setUploadFile}
      csvOptions={csvOptions}
      setCsvOptions={setCsvOptions}
      importResult={importResult}
      setImportResult={setImportResult}
      isUploading={uploadMutation.isPending}
      onUpload={handleFileUpload}
      onClose={handleCloseModal}
      entityName="subscriptions"
      dialogTitle="Import Subscriptions from CSV"
      dialogDescription="Import subscription data from a CSV file. All imports must include headers."
      dialogIcon={<FileSpreadsheetIcon className="h-5 w-5"/>}
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
