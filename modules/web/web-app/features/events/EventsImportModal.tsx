import { FileSpreadsheetIcon } from 'lucide-react'
import { useState } from 'react'
import { z } from 'zod'

import { CSVImportDialog, CSVImportConfig } from '@/components/CSVImportDialog'
import { useBatchJobCreate, CreateBatchJobRequest } from '@/features/batch-jobs/useBatchJobCreate'
import { BatchJobType } from '@/rpc/api/batchjobs/v1/models_pb'

import type { FunctionComponent } from 'react'

const eventsRowSchema = z.object({
  event_code: z.string().min(1, 'Required'),
  customer_id: z.string().min(1, 'Required'),
  timestamp: z.string().refine(
    v => !v || !isNaN(Date.parse(v)),
    'Must be a valid ISO 8601 timestamp'
  ).optional(),
})

type EventsImportOptions = CSVImportConfig

interface EventsImportModalProps {
  openState: [boolean, (open: boolean) => void]
  onSuccess?: () => void
}

export const EventsImportModal: FunctionComponent<EventsImportModalProps> = ({
  openState,
}) => {
  const [isOpen, setIsOpen] = openState
  const [uploadFile, setUploadFile] = useState<File | null>(null)
  const [csvOptions, setCsvOptions] = useState<EventsImportOptions>({
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
          jobType: BatchJobType.EVENT_CSV_IMPORT,
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
    <CSVImportDialog<EventsImportOptions>
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
      entityName="events"
      dialogTitle="Import Events from CSV"
      dialogDescription="Import usage events from a CSV file. All imports must include headers."
      dialogIcon={<FileSpreadsheetIcon className="h-5 w-5" />}
      rowSchema={eventsRowSchema}
      requiredColumns={[
        { name: 'event_code' },
        {
          name: 'customer_id',
          tooltipMessage: 'Meteroid Customer ID or external alias',
        },
      ]}
      optionalColumns={[
        { name: 'event_id', tooltipMessage: 'Auto-generated UUID if empty' },
        { name: 'timestamp', tooltipMessage: 'ISO 8601 format. Defaults to current time.' },
        { name: '(additional)', tooltipMessage: 'Any extra columns become event properties' },
      ]}
    />
  )
}
