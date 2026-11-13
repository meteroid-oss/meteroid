import { create } from '@bufbuild/protobuf'
import { FileSpreadsheetIcon } from 'lucide-react'
import { useState } from 'react'
import { z } from 'zod'

import { CSVImportDialog } from '@/components/CSVImportDialog'
import { useBatchJobCreate, CreateBatchJobRequestSchema } from '@/features/batch-jobs/useBatchJobCreate'
import { useCustomPropertyImportColumn } from '@/features/customproperties/useCustomPropertyImportColumn'
import { BatchJobType } from '@/rpc/api/batchjobs/v1/models_pb'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'

import { customerCsvOptionalColumns, customerCsvRequiredColumns } from './customerCsvColumns'

import type { CsvColumnSpec } from './customerCsvColumns'
import type { ColumnDefinition } from '@/components/csvPreview'
import type { FunctionComponent } from 'react'

const optionalValid = (re: RegExp, msg: string) =>
  z.string().refine(v => !v || re.test(v), msg).optional()

const toColumnDefinition = ({ name, hint, fields }: CsvColumnSpec): ColumnDefinition => ({
  name,
  tooltipMessage: fields ? (
    <div className="space-y-2">
      <p className="font-semibold">Available fields:</p>
      <ul className="list-disc pl-4 space-y-1">
        {fields.map(field => (
          <li key={field}>{field}</li>
        ))}
      </ul>
      {hint && <p>{hint}</p>}
    </div>
  ) : (
    hint
  ),
})

const customerRowSchema = z.object({
  // Blank for INDIVIDUAL rows (derived from first/last name); a blank COMPANY name is rejected on import.
  name: z.string().optional(),
  currency: z.string().regex(/^[A-Z]{3}$/, '3-letter ISO code (e.g., USD, EUR)'),
  customer_type: optionalValid(/^(COMPANY|INDIVIDUAL)$/i, 'Must be COMPANY or INDIVIDUAL'),
  'billing_address.country': optionalValid(/^[A-Z]{2}$/, '2-letter country code (e.g., US, FR)'),
  'shipping_address.country': optionalValid(/^[A-Z]{2}$/, '2-letter country code (e.g., US, FR)'),
  is_tax_exempt: optionalValid(/^(true|false)$/, 'Must be true or false'),
  'tax_rate1.rate': optionalValid(/^\d+(\.\d+)?$/, 'Must be a number'),
  'tax_rate2.rate': optionalValid(/^\d+(\.\d+)?$/, 'Must be a number'),
  preferred_locales: optionalValid(
    /^[a-zA-Z]{2,3}(-[a-zA-Z]{2,4})?(\s*,\s*[a-zA-Z]{2,3}(-[a-zA-Z]{2,4})?)*$/,
    'Comma-separated BCP-47 codes (e.g., fr-FR,en)'
  ),
  invoicing_language: optionalValid(/^[a-zA-Z]{2,3}(-[a-zA-Z]{2,4})?$/, 'BCP-47 code (e.g., en-US, fr-FR)'),
})

interface CustomersImportModalProps {
  openState: [boolean, (open: boolean) => void]
  onSuccess?: () => void
}

export const CustomersImportModal: FunctionComponent<CustomersImportModalProps> = ({
  openState,
}) => {
  const [isOpen, setIsOpen] = openState
  const [uploadFile, setUploadFile] = useState<File | null>(null)
  const [csvOptions, setCsvOptions] = useState({
    delimiter: ',',
  })
  const [batchJobId, setBatchJobId] = useState<string | null>(null)
  const [duplicateDetected, setDuplicateDetected] = useState(false)

  const customPropertyColumn = useCustomPropertyImportColumn(CustomPropertyEntityType.CUSTOMER)

  const createMutation = useBatchJobCreate({
    onSuccess: (jobId) => setBatchJobId(jobId),
    onDuplicate: () => setDuplicateDetected(true),
  })

  const buildRequest = (forceDuplicate = false) => {
    if (!uploadFile) return null
    return uploadFile.arrayBuffer().then(
      buffer =>
        create(CreateBatchJobRequestSchema, {
          fileData: new Uint8Array(buffer),
          jobType: BatchJobType.CUSTOMER_CSV_IMPORT,
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
      entityName="customers"
      dialogTitle="Import Customers from CSV"
      dialogDescription="Import customer data from a CSV file. All imports must include headers."
      dialogIcon={<FileSpreadsheetIcon className="h-5 w-5" />}
      rowSchema={customerRowSchema}
      requiredColumns={customerCsvRequiredColumns.map(toColumnDefinition)}
      optionalColumns={[
        ...customerCsvOptionalColumns.map(toColumnDefinition),
        ...customPropertyColumn,
      ]}
      additionalInfo={
        <ul>
          <li>• Headers are required in the first row</li>
          <li>
            • If a customer with the same alias exists, the row replaces it: blank or missing
            columns are cleared, except preferred_locales and custom properties
          </li>
          <li>
            • Group fields (e.g. <i>billing_address.*</i>): hover on info icon to see available
            fields within the group
          </li>
        </ul>
      }
    />
  )
}
