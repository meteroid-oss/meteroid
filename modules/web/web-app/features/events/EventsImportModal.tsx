import { useMutation } from '@connectrpc/connect-query'
import { Checkbox, Label } from '@md/ui'
import { FileSpreadsheetIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { CSVImportConfig, CSVImportDialog, CSVImportResult } from '@/components/CSVImportDialog'
import { ingestEventsFromCsv } from '@/rpc/api/events/v1/events-EventsIngestionService_connectquery'
import { FileData, IngestEventsFromCsvRequest } from '@/rpc/api/events/v1/events_pb'

import type { FunctionComponent } from 'react'

interface EventsImportModalProps {
  openState: [boolean, (open: boolean) => void]
  onSuccess?: () => void
}

interface EventsCsvOptions extends CSVImportConfig {
  allowBackfilling: boolean
}

export const EventsImportModal: FunctionComponent<EventsImportModalProps> = ({
  openState,
  onSuccess,
}) => {
  const [isOpen, setIsOpen] = openState
  const [uploadFile, setUploadFile] = useState<File | null>(null)
  const [csvOptions, setCsvOptions] = useState<EventsCsvOptions>({
    delimiter: ',',
    allowBackfilling: true,
    failOnError: false,
  })
  const [importResult, setImportResult] = useState<CSVImportResult<string> | null>(null)

  // CSV upload mutation
  const uploadMutation = useMutation(ingestEventsFromCsv, {
    onSuccess: async response => {
      const { totalRows, successfulEvents, failures } = response

      setImportResult({
        totalRows,
        successful: successfulEvents,
        failures:
          failures?.map(f => ({
            rowNumber: f.rowNumber,
            identifier: f.eventId,
            reason: f.reason,
          })) || [],
      })

      // Only close modal and refetch if completely successful
      if (!failures || failures.length === 0) {
        setIsOpen(false)
        setUploadFile(null)
        setImportResult(null)
        onSuccess?.()
        toast.success(`Successfully imported ${successfulEvents} events`)
      }
    },
    onError: error => {
      setImportResult({
        totalRows: 0,
        successful: 0,
        failures: [
          {
            rowNumber: 0,
            identifier: '',
            reason: error.message,
          },
        ],
      })
    },
  })

  const handleFileUpload = async () => {
    if (!uploadFile) return

    setImportResult(null) // Clear previous results
    const buffer = await uploadFile.arrayBuffer()
    const request = new IngestEventsFromCsvRequest({
      file: new FileData({ data: new Uint8Array(buffer) }),
      delimiter: csvOptions.delimiter,
      allowBackfilling: csvOptions.allowBackfilling,
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
      entityName="events"
      identifierLabel="Event ID"
      dialogTitle="Import Events from CSV"
      dialogDescription="Import event data from a CSV file. All imports must include headers."
      dialogIcon={<FileSpreadsheetIcon className="h-5 w-5"/>}
      requiredColumns={['event_code', 'customer_id']}
      optionalColumns={['event_id', 'timestamp', '+ any properties']}
      additionalInfo={
        <>
          • Headers are required in the first row
          <br/>
          • Timestamp should be in ISO 8601 format
          <br/>
          • Additional columns will be stored as event properties
        </>
      }
      additionalOptions={
        <div className="flex items-start space-x-3">
          <div>
            <Checkbox
              id="allowBackfilling"
              checked={csvOptions.allowBackfilling}
              onCheckedChange={checked =>
                setCsvOptions(prev => ({ ...prev, allowBackfilling: checked === true }))
              }
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="allowBackfilling" className="text-sm font-medium">
              Allow backfilling
            </Label>
            <p className="text-xs text-muted-foreground">
              Allow importing events with timestamps in the past.
            </p>
          </div>
        </div>
      }
    />
  )
}
