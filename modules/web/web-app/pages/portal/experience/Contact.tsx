import { contactName } from './screens/creditCopy'

/**
 * Who the customer asks when the portal offers no action: the merchant by name, else "us",
 * as a mail link when the merchant set a contact email on its company.
 */
export const Contact = ({
  name,
  email,
  label,
}: {
  name?: string
  email?: string
  /** Shown instead of the merchant's name, e.g. "contact us". */
  label?: string
}) => {
  const text = label ?? contactName(name)
  if (!email) return <>{text}</>
  return (
    <a href={`mailto:${email}`} style={{ color: 'inherit', textDecoration: 'underline' }}>
      {text}
    </a>
  )
}
