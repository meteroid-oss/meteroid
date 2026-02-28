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
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { AlertCircleIcon, CheckCircleIcon, FileUpIcon, InfoIcon, XCircleIcon } from 'lucide-react'
import { Dispatch, ReactNode, SetStateAction } from 'react'


export interface ColumnDefinition {
  name: string
  tooltipMessage?: ReactNode
}

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
  requiredColumns: ColumnDefinition[]
  optionalColumns: ColumnDefinition[]
  additionalInfo?: ReactNode
  additionalOptions?: ReactNode
  fileMaxSizeBytes?: number // Maximum file size in bytes (default: 10MB)
  showRejectOnError?: boolean

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
  fileMaxSizeBytes = 10 * 1024 * 1024, // Default to 10MB
  showRejectOnError = true,
  entityName,
  identifierLabel,
  dialogTitle,
  dialogDescription,
  dialogIcon,
}: CSVImportDialogProps<TConfig>) {
  const maxSizeMB = fileMaxSizeBytes / (1024 * 1024)
  const tooLargeFile = uploadFile ? uploadFile.size > fileMaxSizeBytes : false

  return (
    <Dialog open={isOpen} onOpenChange={setIsOpen}>
      <DialogContent className="sm:max-w-2xl max-h-[90vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {dialogIcon}
            {dialogTitle}
          </DialogTitle>
          <DialogDescription>{dialogDescription}</DialogDescription>
        </DialogHeader>

        <div className="space-y-6 overflow-y-auto flex-1">
          {!importResult ? (
            <>
              {/* File Upload */}
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <Label className="text-sm font-medium">CSV File</Label>
                  <span className="text-xs text-muted-foreground">
                    Max size: <span className="font-semibold">{maxSizeMB} MB</span>
                  </span>
                </div>
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
                    <div className="mt-2 space-y-2">
                      <div className="text-sm text-muted-foreground">
                        Selected: {uploadFile.name} ({(uploadFile.size / 1024).toFixed(1)} KB)
                      </div>
                      {tooLargeFile && (
                        <div
                          className="flex items-start gap-2 p-3 bg-yellow-50 dark:bg-yellow-950/20 border border-yellow-200 dark:border-yellow-800 rounded-md">
                          <AlertCircleIcon className="h-4 w-4 text-yellow-600 dark:text-yellow-500 mt-0.5 shrink-0"/>
                          <div className="text-sm text-yellow-800 dark:text-yellow-200">
                            File size exceeds maximum allowed limit of {maxSizeMB} MB.
                          </div>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>

              {/* CSV Format Requirements */}
              <Card>
                <CardContent className="p-4 space-y-3">
                  <div className="grid grid-cols-1 gap-3 text-sm">
                    <div className="flex items-start gap-2">
                      <strong className="w-32 shrink-0">Required columns:</strong>
                      <div className="flex gap-2 flex-wrap">
                        <TooltipProvider>
                          {requiredColumns.map(col => (
                            <Badge key={col.name} variant="outline" className="text-xs gap-1">
                              {col.name}
                              {col.tooltipMessage && (
                                <Tooltip>
                                  <TooltipTrigger asChild>
                                    <InfoIcon className="h-3 w-3 text-muted-foreground cursor-help inline-block"/>
                                  </TooltipTrigger>
                                  <TooltipContent className="max-w-xs">
                                    {col.tooltipMessage}
                                  </TooltipContent>
                                </Tooltip>
                              )}
                            </Badge>
                          ))}
                        </TooltipProvider>
                      </div>
                    </div>
                    {optionalColumns.length > 0 && (
                      <div className="flex items-start gap-2">
                        <strong className="w-32 shrink-0">Optional columns:</strong>
                        <div className="flex gap-2 flex-wrap">
                          <TooltipProvider>
                            {optionalColumns.map(col => (
                              <Badge key={col.name} variant="outline" className="text-xs gap-1">
                                {col.name}
                                {col.tooltipMessage && (
                                  <Tooltip>
                                    <TooltipTrigger asChild>
                                      <InfoIcon className="h-3 w-3 text-muted-foreground cursor-help inline-block"/>
                                    </TooltipTrigger>
                                    <TooltipContent className="max-w-xs">
                                      {col.tooltipMessage}
                                    </TooltipContent>
                                  </Tooltip>
                                )}
                              </Badge>
                            ))}
                          </TooltipProvider>
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

                {showRejectOnError && (
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
                )}
              </div>

              {/* Upload Button */}
              <div className="flex gap-2">
                <Button
                  onClick={onUpload}
                  disabled={!uploadFile || isUploading || tooLargeFile}
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
