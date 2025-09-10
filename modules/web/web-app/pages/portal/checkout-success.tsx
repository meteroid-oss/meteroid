import { CheckCircle } from 'lucide-react'
import { useForceTheme } from 'providers/ThemeProvider'

export const PortalCheckoutSuccess = () => {
  useForceTheme('light')

  return (
    <div className="h-full w-full bg-[#00000002]">
      <div className="flex flex-col items-center justify-center h-full max-w-md mx-auto px-6 py-12 text-center">
        <CheckCircle className="h-12 w-12 text-success mb-4 " />
        <h2 className="text-md font-semibold text-gray-800 mb-2">Payment Successful!</h2>
        <p className="text-gray-800 text-sm">
          Thank you for your payment. You can now safely close this tab.
        </p>
      </div>
    </div>
  )
}
