import { useMutation } from '@connectrpc/connect-query'
import { FileSpreadsheetIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

import { CSVImportDialog, CSVImportResult } from '@/components/CSVImportDialog'
import { ingestCsv } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
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
      dialogIcon={<FileSpreadsheetIcon className="h-5 w-5" />}
      requiredColumns={['name', 'currency']}
      optionalColumns={[
        'invoicing_emails',
        'alias',
        'billing_email',
        'phone',
        'invoicing_entity_id',
        'vat_number',
        'is_tax_exempt',
        'tax_rate1.*',
        'tax_rate2.*',
        'billing_address.*',
        'shipping_address.*',
      ]}
      additionalInfo={
        <>
          • currency should be a 3-letter ISO code (e.g., USD, EUR)
          <br />
          • invoicing_emails can be comma-separated for multiple emails
          <br />
          • tax_rate1.* and tax_rate2.*: tax_rate1.tax_code, tax_rate1.name, tax_rate1.rate (and
          same for tax_rate2)
          <br />
          • billing_address.*: billing_address.line1, billing_address.line2, billing_address.city,
          billing_address.country, billing_address.state, billing_address.zip_code
          <br />
          • shipping_address.*: shipping_address.same_as_billing, shipping_address.line1,
          shipping_address.line2, shipping_address.city, shipping_address.country,
          shipping_address.state, shipping_address.zip_code
          <br />
          • shipping_address.same_as_billing: true/false to copy billing address
          <br />
          • if customer with the same alias exists, it will be updated
        </>
      }
    />
  )
}
