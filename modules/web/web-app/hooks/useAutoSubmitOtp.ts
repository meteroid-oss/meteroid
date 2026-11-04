import { useEffect, useRef } from 'react'

/**
 * Submits an OTP form automatically once a full 6-digit code is entered.
 * Recovery codes are alphanumeric with dashes, so they never match and still
 * require an explicit submit. The ref keeps submission idempotent so re-renders
 * don't fire it twice; it resets once the code drops below a full 6 digits.
 */
export const useAutoSubmitOtp = (code: string, isPending: boolean, submit: () => void) => {
  const submitted = useRef(false)

  useEffect(() => {
    if (!/^\d{6}$/.test(code)) {
      submitted.current = false
      return
    }
    if (!isPending && !submitted.current) {
      submitted.current = true
      submit()
    }
  }, [code, isPending, submit])
}
