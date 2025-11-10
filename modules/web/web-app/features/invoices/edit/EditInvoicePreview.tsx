import { Spinner } from '@md/ui'
import { useEffect, useState } from 'react'

interface EditInvoicePreviewProps {
  previewData: string[]
  isLoading: boolean
}

export const EditInvoicePreview = ({ previewData, isLoading }: EditInvoicePreviewProps) => {
  const [displayedSvgs, setDisplayedSvgs] = useState<string[]>([])

  useEffect(() => {
    if (!isLoading && previewData.length > 0) {
      setDisplayedSvgs(previewData)
    } else if (displayedSvgs.length === 0 && previewData.length > 0) {
      setDisplayedSvgs(previewData)
    }
  }, [isLoading, previewData])

  if (displayedSvgs.length === 0) {
    return (
      <div className="h-full flex items-center justify-center bg-gray-50 rounded-lg border-2 border-dashed border-gray-300">
        <div className="text-center">
          <div className="text-lg font-medium text-gray-500 mb-2">Invoice Preview</div>
          <div className="text-sm text-gray-400">
            {isLoading
              ? 'Generating preview...'
              : 'Make changes to see the updated invoice preview'}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="w-full h-full flex flex-col relative">
      <div className="flex flex-col items-center justify-center gap-5 bg-gray-100 py-10 relative">
        {isLoading && (
          <div className="absolute inset-0 flex items-center justify-center z-10">
            <div className="absolute inset-0 backdrop-blur-[2px] flex items-center justify-center z-10"></div>
            <div className="absolute inset-0 flex items-center justify-center z-10 text-muted-foreground">
              <Spinner />
            </div>
          </div>
        )}

        {displayedSvgs.map((svgContent, i) => (
          <div
            className="bg-white"
            key={`svg-${i}`}
            style={{
              boxShadow: '0px 4px 12px rgba(89, 85, 101, .2)',
            }}
            dangerouslySetInnerHTML={{ __html: svgContent }}
          />
        ))}
      </div>
    </div>
  )
}
