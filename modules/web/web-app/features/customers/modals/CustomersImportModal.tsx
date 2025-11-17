import { useMutation } from '@connectrpc/connect-query'
import { FileSpreadsheetIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { CSVImportDialog, CSVImportResult } from '@/components/CSVImportDialog'
import { ingestCsv } from '@/rpc/api/customers/v1/customers-CustomersIngestService_connectquery'
import { FileData, IngestCsvRequest } from '@/rpc/api/customers/v1/customers_pb'

import type { FunctionComponent } from 'react'

interface CustomersImportModalProps {
  openState: [boolean, (open: boolean) => void]
  onSuccess?: () => void
}

export const CustomersImportModal: FunctionComponent<CustomersImportModalProps> = ({
  openState,
  onSuccess,
}) => {
  const [isOpen, setIsOpen] = openState
  const [uploadFile, setUploadFile] = useState<File | null>(null)
  const [csvOptions, setCsvOptions] = useState({
    delimiter: ',',
    failOnError: false,
  })
  const [importResult, setImportResult] = useState<CSVImportResult<string> | null>(null)

  // CSV upload mutation
  const uploadMutation = useMutation(ingestCsv, {
    onSuccess: async response => {
      const { totalRows, successfulRows, failures } = response

      setImportResult({
        totalRows,
        successful: successfulRows,
        failures:
          failures?.map(f => ({
            rowNumber: f.rowNumber,
            identifier: f.customerAlias,
            reason: f.reason,
          })) || [],
      })

      // Only close modal and refetch if completely successful
      if (!failures || failures.length === 0) {
        setIsOpen(false)
        setUploadFile(null)
        setImportResult(null)
        onSuccess?.()
        toast.success(`Successfully imported ${successfulRows} customers`)
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
      entityName="customers"
      identifierLabel="Customer Alias"
      dialogTitle="Import Customers from CSV"
      dialogDescription="Import customer data from a CSV file. All imports must include headers."
      dialogIcon={<FileSpreadsheetIcon className="h-5 w-5"/>}
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
        <>
          <ul>
            <li>• Headers are required in the first row</li>
            <li>• If customer with the same alias exists, it will be updated</li>
            <li>• Group fields (e.g. <i>billing_address.*</i>): hover on info icon to see available fields within the
              group
            </li>
          </ul>
        </>
      }
    />
  )
}
