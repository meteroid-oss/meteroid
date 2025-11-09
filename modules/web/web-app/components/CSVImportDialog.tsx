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
import { AlertCircleIcon, CheckCircleIcon, FileUpIcon, XCircleIcon } from 'lucide-react'
import { Dispatch, ReactNode, SetStateAction } from 'react'


export interface CSVImportConfig {
  delimiter: string
  failOnError: boolean

  [key: string]: unknown // Allow for additional options
}

export interface CSVImportResult<TIdentifier = string> {
  totalRows: number
  successful: number
  failures: Array<{
    rowNumber: number
    identifier: TIdentifier
    reason: string
  }>
}

export interface CSVImportDialogProps<TConfig extends CSVImportConfig> {
  // Dialog state
  isOpen: boolean
  setIsOpen: (open: boolean) => void

  // File upload state
  uploadFile: File | null
  setUploadFile: (file: File | null) => void

  // CSV options
  csvOptions: TConfig
  setCsvOptions: Dispatch<SetStateAction<TConfig>>

  // Import results
  importResult: CSVImportResult | null
  setImportResult: (result: CSVImportResult | null) => void

  // Mutation state
  isUploading: boolean

  // Handlers
  onUpload: () => void
  onClose: () => void

  // Configuration
  requiredColumns: string[]
  optionalColumns: string[]
  additionalInfo?: ReactNode
  additionalOptions?: ReactNode

  // Labels
  entityName: string // e.g., "customers", "events"
  identifierLabel?: string // e.g., "Customer Alias", "Event ID"
  dialogTitle: string // e.g., "Import Customers from CSV"
  dialogDescription: string // e.g., "Import customer data from a CSV file."
  dialogIcon?: ReactNode // Optional icon for the dialog title
}

export function CSVImportDialog<TConfig extends CSVImportConfig = CSVImportConfig>({
  isOpen,
  setIsOpen,
  uploadFile,
  setUploadFile,
  csvOptions,
  setCsvOptions,
  importResult,
  setImportResult,
  isUploading,
  onUpload,
  onClose,
  requiredColumns,
  optionalColumns,
  additionalInfo,
  additionalOptions,
  entityName,
  identifierLabel,
  dialogTitle,
  dialogDescription,
  dialogIcon,
}: CSVImportDialogProps<TConfig>) {
  return (
    <Dialog open={isOpen} onOpenChange={setIsOpen}>
      <DialogContent className="sm:max-w-2xl max-h-[90vh]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {dialogIcon}
            {dialogTitle}
          </DialogTitle>
          <DialogDescription>{dialogDescription}</DialogDescription>
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
                        {requiredColumns.map(col => (
                          <Badge key={col} variant="outline" className="text-xs">
                            {col}
                          </Badge>
                        ))}
                      </div>
                    </div>
                    {optionalColumns.length > 0 && (
                      <div className="grid grid-cols-2">
                        <strong>Optional columns:</strong>
                        <div className="mt-1 flex gap-2 flex-wrap">
                          {optionalColumns.map(col => (
                            <Badge key={col} variant="outline" className="text-xs">
                              {col}
                            </Badge>
                          ))}
                        </div>
                      </div>
                    )}
                    {additionalInfo && <div className="text-xs text-muted-foreground">{additionalInfo}</div>}
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

                {additionalOptions}

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
                      Reject the entire import if any row contains an error. Otherwise, import valid
                      rows and report errors.
                    </p>
                  </div>
                </div>
              </div>

              {/* Upload Button */}
              <div className="flex gap-2">
                <Button
                  onClick={onUpload}
                  disabled={!uploadFile || isUploading}
                  className="flex-1"
                  size="lg"
                >
                  {isUploading ? (
                    <>
                      <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white mr-2"/>
                      Processing...
                    </>
                  ) : (
                    <>
                      <FileUpIcon className="h-4 w-4 mr-2"/>
                      Import {entityName}
                    </>
                  )}
                </Button>
                <Button variant="outline" onClick={onClose}>
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
                            {importResult.successful} of {importResult.totalRows} {entityName} imported
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
                                    {failure.identifier && (
                                      <Badge variant="outline" className="text-xs font-mono">
                                        {identifierLabel && `${identifierLabel}: `}
                                        {String(failure.identifier)}
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
                  <Button variant="ghost" onClick={onClose}>
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
