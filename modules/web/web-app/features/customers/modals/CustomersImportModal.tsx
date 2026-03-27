import { FileSpreadsheetIcon } from 'lucide-react'
import { useState } from 'react'
import { z } from 'zod'

import { CSVImportDialog } from '@/components/CSVImportDialog'
import { useBatchJobCreate, CreateBatchJobRequest } from '@/features/batch-jobs/useBatchJobCreate'
import { BatchJobType } from '@/rpc/api/batchjobs/v1/models_pb'

import type { FunctionComponent } from 'react'

const optionalValid = (re: RegExp, msg: string) =>
  z.string().refine(v => !v || re.test(v), msg).optional()

const customerRowSchema = z.object({
  name: z.string().min(1, 'Name is required'),
  currency: z.string().regex(/^[A-Z]{3}$/, '3-letter ISO code (e.g., USD, EUR)'),
  'billing_address.country': optionalValid(/^[A-Z]{2}$/, '2-letter country code (e.g., US, FR)'),
  'shipping_address.country': optionalValid(/^[A-Z]{2}$/, '2-letter country code (e.g., US, FR)'),
  is_tax_exempt: optionalValid(/^(true|false)$/, 'Must be true or false'),
  'tax_rate1.rate': optionalValid(/^\d+(\.\d+)?$/, 'Must be a number'),
  'tax_rate2.rate': optionalValid(/^\d+(\.\d+)?$/, 'Must be a number'),
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
      requiredColumns={[
        { name: 'name' },
        { name: 'currency', tooltipMessage: '3-letter ISO code (e.g., USD, EUR)' },
      ]}
      optionalColumns={[
        { name: 'invoicing_emails', tooltipMessage: 'Comma-separated for multiple emails' },
        { name: 'alias' },
        { name: 'billing_email' },
        { name: 'phone' },
        { name: 'invoicing_entity_id', tooltipMessage: '`default` entity is used if empty' },
        { name: 'vat_number' },
        { name: 'is_tax_exempt' },
        {
          name: 'tax_rate1.*',
          tooltipMessage: (
            <div className="space-y-2">
              <p className="font-semibold">Available fields:</p>
              <ul className="list-disc pl-4 space-y-1">
                <li>tax_rate1.tax_code</li>
                <li>tax_rate1.name</li>
                <li>tax_rate1.rate</li>
              </ul>
            </div>
          ),
        },
        {
          name: 'tax_rate2.*',
          tooltipMessage: (
            <div className="space-y-2">
              <p className="font-semibold">Available fields:</p>
              <ul className="list-disc pl-4 space-y-1">
                <li>tax_rate2.tax_code</li>
                <li>tax_rate2.name</li>
                <li>tax_rate2.rate</li>
              </ul>
            </div>
          ),
        },
        {
          name: 'billing_address.*',
          tooltipMessage: (
            <div className="space-y-2">
              <p className="font-semibold">Available fields:</p>
              <ul className="list-disc pl-4 space-y-1">
                <li>billing_address.line1</li>
                <li>billing_address.line2</li>
                <li>billing_address.city</li>
                <li>billing_address.country</li>
                <li>billing_address.state</li>
                <li>billing_address.zip_code</li>
              </ul>
            </div>
          ),
        },
        {
          name: 'shipping_address.*',
          tooltipMessage: (
            <div className="space-y-2">
              <p className="font-semibold">Available fields:</p>
              <ul className="list-disc pl-4 space-y-1">
                <li>shipping_address.same_as_billing (true/false)</li>
                <li>shipping_address.line1</li>
                <li>shipping_address.line2</li>
                <li>shipping_address.city</li>
                <li>shipping_address.country</li>
                <li>shipping_address.state</li>
                <li>shipping_address.zip_code</li>
              </ul>
            </div>
          ),
        },
      ]}
      additionalInfo={
        <ul>
          <li>• Headers are required in the first row</li>
          <li>• If customer with the same alias exists, it will be updated</li>
          <li>
            • Group fields (e.g. <i>billing_address.*</i>): hover on info icon to see available
            fields within the group
          </li>
        </ul>
      }
    />
  )
}
