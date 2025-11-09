import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Card,
  CardContent,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  ScrollArea,
} from '@md/ui'
import { AlertCircleIcon, CheckCircleIcon, FileSpreadsheetIcon, FileUpIcon, XCircleIcon } from 'lucide-react'
import { useState } from 'react'
import { toast } from 'sonner'

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
  const [importResult, setImportResult] = useState<{
    totalRows: number
    successful: number
    failures: Array<{ rowNumber: number; customerAlias: string; reason: string }>
  } | null>(null)

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
            customerAlias: f.customerAlias,
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
            customerAlias: '',
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
    <Dialog open={isOpen} onOpenChange={setIsOpen}>
      <DialogContent className="sm:max-w-2xl max-h-[90vh]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <FileSpreadsheetIcon className="h-5 w-5"/>
            Import Customers from CSV
          </DialogTitle>
          <DialogDescription>
            Import customer data from a CSV file. All imports must include headers.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-6">
          {!importResult ? (
            <>
              {/* File Upload */}
              <div className="space-y-3">
                <Label className="text-sm font-medium">CSV File</Label>
                <div
                  className="border-2 border-dashed border-muted-foreground/25 rounded-lg p-4 hover:border-muted-foreground/50 transition-colors">
                  <Input
                    id="file"
                    type="file"
                    accept=".csv,.txt"
                    onChange={e => setUploadFile(e.target.files?.[0] || null)}
                    className="cursor-pointer"
                  />
                  {uploadFile && (
                    <div className="mt-2 text-sm text-muted-foreground">
                      Selected: {uploadFile.name} ({(uploadFile.size / 1024).toFixed(1)} KB)
                    </div>
                  )}
                </div>
              </div>

              {/* CSV Format Requirements */}
              <Card>
                <CardContent className="p-4 space-y-3">
                  <div className="grid grid-cols-1 gap-3 text-sm">
                    <div className="grid grid-cols-2">
                      <strong>Required columns:</strong>
                      <div className="mt-1 flex gap-2 flex-wrap">
                        <Badge variant="outline" className="text-xs">
                          name
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          currency
                        </Badge>
                      </div>
                    </div>
                    <div className="grid grid-cols-2">
                      <strong>Optional columns:</strong>
                      <div className="mt-1 flex gap-2 flex-wrap">
                        <Badge variant="outline" className="text-xs">
                          invoicing_emails
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          alias
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          billing_email
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          phone
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          invoicing_entity_id
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          vat_number
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          is_tax_exempt
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          tax_rate1.*
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          tax_rate2.*
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          billing_address.*
                        </Badge>
                        <Badge variant="outline" className="text-xs">
                          shipping_address.*
                        </Badge>
                      </div>
                    </div>
                    <div className="text-xs text-muted-foreground">
                      • currency should be a 3-letter ISO code (e.g., USD, EUR)
                      <br/>
                      • invoicing_emails can be comma-separated for multiple emails
                      <br/>
                      • tax_rate1.* and tax_rate2.*: tax_rate1.tax_code, tax_rate1.name, tax_rate1.rate (and same for
                      tax_rate2)
                      <br/>
                      • billing_address.*: billing_address.line1, billing_address.line2, billing_address.city,
                      billing_address.country, billing_address.state, billing_address.zip_code
                      <br/>
                      • shipping_address.*: shipping_address.same_as_billing, shipping_address.line1,
                      shipping_address.line2, shipping_address.city, shipping_address.country,
                      shipping_address.state, shipping_address.zip_code
                      <br/>
                      • shipping_address.same_as_billing: true/false to copy billing address
                      <br/>
                      • if customer with the same alias exists, it will be updated
                    </div>
                  </div>
                </CardContent>
              </Card>

              {/* Configuration */}
              <div className="grid grid-cols-1 gap-4">
                <div className="flex items-center space-x-3">
                  <Label htmlFor="delimiter" className="text-sm font-medium min-w-[80px]">
                    Delimiter
                  </Label>
                  <Input
                    id="delimiter"
                    value={csvOptions.delimiter}
                    onChange={e =>
                      setCsvOptions(prev => ({ ...prev, delimiter: e.target.value }))
                    }
                    placeholder=","
                    maxLength={1}
                    className="w-20"
                  />
                </div>

                <div className="flex items-start space-x-3">
                  <div>
                    <Checkbox
                      id="failOnError"
                      checked={csvOptions.failOnError}
                      onCheckedChange={checked =>
                        setCsvOptions(prev => ({ ...prev, failOnError: checked === true }))
                      }
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="failOnError" className="text-sm font-medium">
                      Reject on error
                    </Label>
                    <p className="text-xs text-muted-foreground">
                      Reject the entire import if any row contains an error. Otherwise, import
                      valid rows and report errors.
                    </p>
                  </div>
                </div>
              </div>

              {/* Upload Button */}
              <div className="flex gap-2">
                <Button
                  onClick={handleFileUpload}
                  disabled={!uploadFile || uploadMutation.isPending}
                  className="flex-1"
                  size="lg"
                >
                  {uploadMutation.isPending ? (
                    <>
                      <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white mr-2"/>
                      Processing...
                    </>
                  ) : (
                    <>
                      <FileUpIcon className="h-4 w-4 mr-2"/>
                      Import Customers
                    </>
                  )}
                </Button>
                <Button variant="outline" onClick={handleCloseModal}>
                  Cancel
                </Button>
              </div>
            </>
          ) : (
            <div className="space-y-4">
              <div className="space-y-4">
                {/* Summary */}
                <Card>
                  <CardContent className="p-4">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-2">
                        {importResult.failures.length === 0 ? (
                          <CheckCircleIcon className="h-5 w-5 text-green-500"/>
                        ) : importResult.successful === 0 ? (
                          <XCircleIcon className="h-5 w-5 text-red-500"/>
                        ) : (
                          <AlertCircleIcon className="h-5 w-5 text-yellow-500"/>
                        )}
                        <div>
                          <h3 className="font-medium">
                            {importResult.failures.length === 0
                              ? 'Import Successful'
                              : importResult.successful === 0
                                ? 'Import Failed'
                                : 'Import Partially Successful'}
                          </h3>
                          <p className="text-sm text-muted-foreground">
                            {importResult.successful} of {importResult.totalRows} customers
                            imported
                            {importResult.failures.length > 0 &&
                              `, ${importResult.failures.length} failed`}
                          </p>
                        </div>
                      </div>
                    </div>
                  </CardContent>
                </Card>

                {/* Errors */}
                {importResult.failures.length > 0 && (
                  <Card>
                    <CardContent className="p-4">
                      <div className="mb-2 text-sm font-medium text-muted-foreground">
                        Errors ({importResult.failures.length})
                      </div>
                      <ScrollArea className="h-[300px] pr-4">
                        <div className="space-y-2">
                          {importResult.failures.map((failure, index) => (
                            <div
                              key={index}
                              className="p-3 border rounded-lg bg-red-50 border-red-200 dark:bg-red-950/50 dark:border-red-800"
                            >
                              <div className="flex items-start justify-between">
                                <div className="flex-1 min-w-0">
                                  <div className="flex items-center gap-2 text-sm font-medium mb-1 flex-wrap">
                                    <Badge variant="destructive" className="text-xs">
                                      Row {failure.rowNumber}
                                    </Badge>
                                    {failure.customerAlias && (
                                      <Badge variant="outline" className="text-xs font-mono">
                                        {failure.customerAlias}
                                      </Badge>
                                    )}
                                  </div>
                                  <p className="text-sm text-red-700 dark:text-red-300 break-words">
                                    {failure.reason}
                                  </p>
                                </div>
                              </div>
                            </div>
                          ))}
                        </div>
                      </ScrollArea>
                    </CardContent>
                  </Card>
                )}

                {/* Actions */}
                <div className="flex gap-2">
                  <Button
                    variant="outline"
                    onClick={() => {
                      setImportResult(null)
                      setUploadFile(null)
                    }}
                  >
                    Import Another File
                  </Button>
                  <Button variant="ghost" onClick={handleCloseModal}>
                    Close
                  </Button>
                </div>
              </div>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
