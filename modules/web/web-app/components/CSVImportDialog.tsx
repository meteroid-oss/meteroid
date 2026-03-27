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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import {
  AlertCircleIcon,
  CheckCircle2Icon,
  CircleAlertIcon,
  FileUpIcon,
  InfoIcon,
  XCircleIcon,
} from 'lucide-react'
import { Dispatch, ReactNode, SetStateAction, useCallback, useEffect, useState } from 'react'
import { z } from 'zod'

import { BatchJobProgress } from '@/features/batch-jobs/BatchJobProgress'

import { CSVPreview, ColumnDefinition, matchesAnyColumnDef, parseCSVPreview } from './csvPreview'

export type { ColumnDefinition }

export interface CSVImportConfig {
  delimiter: string
  failOnError?: boolean

  [key: string]: unknown
}

export interface CSVImportDialogProps<TConfig extends CSVImportConfig> {
  isOpen: boolean
  setIsOpen: (open: boolean) => void

  uploadFile: File | null
  setUploadFile: (file: File | null) => void

  csvOptions: TConfig
  setCsvOptions: Dispatch<SetStateAction<TConfig>>

  isUploading: boolean
  batchJobId?: string | null

  onUpload: () => void
  onForceUpload?: () => void
  onClose: () => void

  duplicateDetected?: boolean

  requiredColumns: ColumnDefinition[]
  optionalColumns: ColumnDefinition[]
  additionalInfo?: ReactNode
  additionalOptions?: ReactNode
  fileMaxSizeBytes?: number
  showRejectOnError?: boolean

  rowSchema?: z.ZodObject<z.ZodRawShape>

  entityName: string
  dialogTitle: string
  dialogDescription: string
  dialogIcon?: ReactNode
}

export function CSVImportDialog<TConfig extends CSVImportConfig = CSVImportConfig>({
  isOpen,
  setIsOpen,
  uploadFile,
  setUploadFile,
  csvOptions,
  setCsvOptions,
  isUploading,
  batchJobId,
  onUpload,
  onForceUpload,
  onClose,
  duplicateDetected = false,
  requiredColumns,
  optionalColumns,
  additionalInfo,
  additionalOptions,
  fileMaxSizeBytes = 10 * 1024 * 1024,
  showRejectOnError = false,
  rowSchema,
  entityName,
  dialogTitle,
  dialogDescription,
  dialogIcon,
}: CSVImportDialogProps<TConfig>) {
  const maxSizeMB = fileMaxSizeBytes / (1024 * 1024)
  const tooLargeFile = uploadFile ? uploadFile.size > fileMaxSizeBytes : false

  const [preview, setPreview] = useState<CSVPreview | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)

  const runPreview = useCallback(async () => {
    if (!uploadFile || tooLargeFile) {
      setPreview(null)
      setPreviewError(null)
      return
    }
    try {
      const result = await parseCSVPreview(
        uploadFile,
        csvOptions.delimiter,
        requiredColumns,
        optionalColumns,
        rowSchema
      )
      setPreview(result)
      setPreviewError(null)
    } catch {
      setPreview(null)
      setPreviewError('Failed to parse file. Check delimiter and file encoding.')
    }
  }, [uploadFile, csvOptions.delimiter, tooLargeFile])

  useEffect(() => {
    runPreview()
  }, [runPreview])

  useEffect(() => {
    if (!isOpen) {
      setPreview(null)
      setPreviewError(null)
    }
  }, [isOpen])

  const hasMissingRequired = (preview?.headerValidation.missingRequired.length ?? 0) > 0

  return (
    <Dialog
      open={isOpen}
      onOpenChange={open => {
        if (!open) {
          onClose()
        } else {
          setIsOpen(true)
        }
      }}
    >
      <DialogContent className="sm:max-w-2xl max-h-[90vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {dialogIcon}
            {dialogTitle}
          </DialogTitle>
          <DialogDescription>{dialogDescription}</DialogDescription>
        </DialogHeader>

        <div className="space-y-6 overflow-y-auto flex-1">
          {batchJobId ? (
            <div className="space-y-4">
              <BatchJobProgress jobId={batchJobId} />
              <Button variant="outline" onClick={onClose} className="w-full">
                Close
              </Button>
            </div>
          ) : (
            <>
              {/* File Upload */}
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <Label className="text-sm font-medium">CSV File</Label>
                  <span className="text-xs text-muted-foreground">
                    Max size: <span className="font-semibold">{maxSizeMB} MB</span>
                  </span>
                </div>
                <div className="border-2 border-dashed border-muted-foreground/25 rounded-lg p-4 hover:border-muted-foreground/50 transition-colors">
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
                        <div className="flex items-start gap-2 p-3 bg-yellow-50 dark:bg-yellow-950/20 border border-yellow-200 dark:border-yellow-800 rounded-md">
                          <AlertCircleIcon className="h-4 w-4 text-yellow-600 dark:text-yellow-500 mt-0.5 shrink-0" />
                          <div className="text-sm text-yellow-800 dark:text-yellow-200">
                            File size exceeds maximum allowed limit of {maxSizeMB} MB.
                          </div>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>

              {/* Delimiter config (before preview so changes re-parse immediately) */}
              <div className="flex items-center space-x-3">
                <Label htmlFor="delimiter" className="text-sm font-medium min-w-[80px]">
                  Delimiter
                </Label>
                <Input
                  id="delimiter"
                  value={csvOptions.delimiter}
                  onChange={e => setCsvOptions(prev => ({ ...prev, delimiter: e.target.value }))}
                  placeholder=","
                  maxLength={1}
                  className="w-20"
                />
              </div>

              {/* Inline preview — appears automatically after file is selected */}
              {uploadFile && !tooLargeFile && preview && (
                <CSVPreviewSection
                  preview={preview}
                  requiredColumns={requiredColumns}
                  entityName={entityName}
                />
              )}

              {previewError && (
                <div className="flex items-start gap-2 p-3 border border-destructive/50 rounded-md bg-destructive/5">
                  <XCircleIcon className="h-4 w-4 text-destructive mt-0.5 shrink-0" />
                  <span className="text-sm text-destructive">{previewError}</span>
                </div>
              )}

              {/* CSV Format Requirements (collapsed when preview is showing) */}
              {(!preview || !uploadFile) && (
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
                                      <span className="inline-flex cursor-help">
                                        <InfoIcon className="h-3 w-3 text-muted-foreground" />
                                      </span>
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
                                        <span className="inline-flex cursor-help">
                                          <InfoIcon className="h-3 w-3 text-muted-foreground" />
                                        </span>
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
                    </div>
                  </CardContent>
                </Card>
              )}

              {/* Additional options */}
              <div className="grid grid-cols-1 gap-4">
                {additionalOptions}

                {showRejectOnError && (
                  <div className="flex items-start space-x-3">
                    <div>
                      <Checkbox
                        id="failOnError"
                        checked={csvOptions.failOnError ?? false}
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
                        Reject the entire chunk if any row contains an error. Otherwise, import all
                        valid rows and report errors.
                      </p>
                    </div>
                  </div>
                )}
              </div>

              {additionalInfo && (
                <div className="text-xs text-muted-foreground">{additionalInfo}</div>
              )}

              {duplicateDetected && (
                <div className="flex items-start gap-2 p-3 border border-yellow-200 dark:border-yellow-800 rounded-md bg-yellow-50 dark:bg-yellow-950/20">
                  <AlertCircleIcon className="h-4 w-4 text-yellow-600 dark:text-yellow-500 mt-0.5 shrink-0" />
                  <div className="text-sm text-yellow-800 dark:text-yellow-200">
                    This file has already been imported. You can import it again if needed.
                  </div>
                </div>
              )}

              {/* Upload Button */}
              <div className="flex gap-2">
                {duplicateDetected && onForceUpload ? (
                  <Button
                    onClick={onForceUpload}
                    disabled={isUploading}
                    variant="destructive"
                    className="flex-1"
                    size="lg"
                  >
                    {isUploading ? (
                      <>
                        <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white mr-2" />
                        Processing...
                      </>
                    ) : (
                      <>
                        <FileUpIcon className="h-4 w-4 mr-2" />
                        Import anyway
                      </>
                    )}
                  </Button>
                ) : (
                  <Button
                    onClick={onUpload}
                    disabled={!uploadFile || isUploading || tooLargeFile || hasMissingRequired}
                    className="flex-1"
                    size="lg"
                  >
                    {isUploading ? (
                      <>
                        <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-white mr-2" />
                        Processing...
                      </>
                    ) : (
                      <>
                        <FileUpIcon className="h-4 w-4 mr-2" />
                        Import {preview ? `~${preview.totalRowEstimate.toLocaleString()} ` : ''}
                        {entityName}
                      </>
                    )}
                  </Button>
                )}
                <Button variant="outline" onClick={onClose}>
                  Cancel
                </Button>
              </div>
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}

function CSVPreviewSection({
  preview,
  requiredColumns,
  entityName,
}: {
  preview: CSVPreview
  requiredColumns: ColumnDefinition[]
  entityName: string
}) {
  const { headers, rows, totalRowEstimate, headerValidation, rowErrors } = preview
  const { missingRequired, unknown } = headerValidation

  const errorMap = new Map<string, string>()
  const errorColumnIndices = new Set<number>()
  for (const err of rowErrors) {
    errorMap.set(`${err.rowIndex}:${err.columnIndex}`, err.message)
    errorColumnIndices.add(err.columnIndex)
  }
  const errorCount = rowErrors.length
  const errorColumnNames = [...errorColumnIndices].map(i => headers[i]).filter(Boolean)

  if (headers.length === 0) {
    return (
      <div className="flex items-start gap-2 p-3 border border-destructive/50 rounded-md bg-destructive/5">
        <XCircleIcon className="h-4 w-4 text-destructive mt-0.5 shrink-0" />
        <span className="text-sm text-destructive">
          No headers found. Check the file format and delimiter.
        </span>
      </div>
    )
  }



  const unknownSet = new Set(unknown)

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <span className="text-sm font-medium">Preview</span>
        <span className="text-xs text-muted-foreground">
          ~{totalRowEstimate.toLocaleString()} {entityName}
        </span>
      </div>

       { headers.length === 1 ? (
      <div className="flex items-start gap-2 p-3 border border-destructive/50 rounded-md bg-destructive/5">
      <XCircleIcon className="h-4 w-4 text-destructive mt-0.5 shrink-0" />
      <span className="text-sm text-destructive">
      Single header found. Is the delimiter correct ?
      </span>
      </div>
      ) : <>
         {missingRequired.length > 0 && (
           <div className="flex items-start gap-2 p-3 border border-destructive/50 rounded-md bg-destructive/5">
             <XCircleIcon className="h-4 w-4 text-destructive mt-0.5 shrink-0" />
             <div className="text-sm">
               <span className="font-medium text-destructive">Missing required columns: </span>
               <span className="text-destructive">{missingRequired.join(', ')}</span>
             </div>
           </div>
         )}

         {unknown.length > 0 && (
           <div className="flex items-start gap-2 p-3 border border-yellow-200 dark:border-yellow-800 rounded-md bg-yellow-50 dark:bg-yellow-950/20">
             <CircleAlertIcon className="h-4 w-4 text-yellow-600 dark:text-yellow-500 mt-0.5 shrink-0" />
             <div className="text-sm text-yellow-800 dark:text-yellow-200">
               Unknown columns (will be ignored): {unknown.join(', ')}
             </div>
           </div>
         )}

       </>
    }



      {missingRequired.length === 0 && unknown.length === 0 && errorCount === 0 && (
        <div className="flex items-center gap-2 p-3 border border-green-200 dark:border-green-800 rounded-md bg-green-50 dark:bg-green-950/20">
          <CheckCircle2Icon className="h-4 w-4 text-green-600 dark:text-green-500 shrink-0" />
          <span className="text-sm text-green-800 dark:text-green-200">
            All required columns present
          </span>
        </div>
      )}

      {errorCount > 0 && (
        <div className="flex items-start gap-2 p-3 border border-yellow-200 dark:border-yellow-800 rounded-md bg-yellow-50 dark:bg-yellow-950/20">
          <CircleAlertIcon className="h-4 w-4 text-yellow-600 dark:text-yellow-500 mt-0.5 shrink-0" />
          <div className="text-sm text-yellow-800 dark:text-yellow-200">
            {errorCount} validation {errorCount === 1 ? 'issue' : 'issues'} in{' '}
            {errorColumnNames.length === 1 ? 'column' : 'columns'}:{' '}
            <span className="font-medium">{errorColumnNames.join(', ')}</span> (scroll table to see
            highlights)
          </div>
        </div>
      )}

      {rows.length > 0 && (
        <div className="max-h-[240px] overflow-auto border rounded-md">
          <Table containerClassName="overflow-visible">
            <TableHeader>
              <TableRow>
                <TableHead className="w-10 text-center text-xs">#</TableHead>
                {headers.map((h, i) => {
                  const isRequired = matchesAnyColumnDef(h, requiredColumns)
                  const isUnknown = unknownSet.has(h)
                  return (
                    <TableHead
                      key={i}
                      className={`text-xs whitespace-nowrap ${isUnknown ? 'text-yellow-600 dark:text-yellow-500' : ''}`}
                    >
                      <span className="flex items-center gap-1">
                        {h}
                        {isRequired && (
                          <CheckCircle2Icon className="h-3 w-3 text-green-500 shrink-0" />
                        )}
                      </span>
                    </TableHead>
                  )
                })}
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row, rowIdx) => (
                <TableRow key={rowIdx}>
                  <TableCell className="text-center text-xs text-muted-foreground tabular-nums">
                    {rowIdx + 1}
                  </TableCell>
                  {headers.map((_, colIdx) => {
                    const cellError = errorMap.get(`${rowIdx}:${colIdx}`)
                    return (
                      <TableCell
                        key={colIdx}
                        className={`text-xs max-w-[200px] truncate ${cellError ? 'bg-red-50 dark:bg-red-950/30 text-red-700 dark:text-red-300' : ''}`}
                      >
                        {cellError ? (
                          <TooltipProvider>
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <span className="cursor-help border-b border-dashed border-red-400">
                                  {row[colIdx] || (
                                    <span className="italic text-muted-foreground">empty</span>
                                  )}
                                </span>
                              </TooltipTrigger>
                              <TooltipContent className="max-w-xs text-xs">
                                {cellError}
                              </TooltipContent>
                            </Tooltip>
                          </TooltipProvider>
                        ) : (
                          (row[colIdx] ?? '')
                        )}
                      </TableCell>
                    )
                  })}
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}

      {totalRowEstimate > rows.length && (
        <p className="text-xs text-muted-foreground text-center">
          Showing {rows.length} of ~{totalRowEstimate.toLocaleString()} rows
        </p>
      )}
    </div>
  )
}
