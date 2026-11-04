// `SkipReason` in meteroid-store `services/customer_emails`, shared by email and reminder skips.
export const EMAIL_SKIP_REASONS: Record<string, string> = {
  emails_off: 'Customer emails are off for this workspace',
  tenant_disabled: 'Customer emails are off for this workspace',
  test_mode: 'Test mode: no allowed recipient',
  kind_disabled: 'This email type is turned off',
  no_recipients: 'No recipient email address',
  not_payable: 'Invoice no longer payable',
  audience_excluded: 'Excluded by the email’s "Send to" rules',
  stopped: 'Reminders stopped for this invoice',
  paused: 'Reminders paused for this invoice',
  opted_out: 'Customer opted out of payment reminders',
  step_removed: 'Reminder step removed from the sequence',
  superseded: 'Superseded by a newer reminder schedule',
  payment_pending: 'A payment was still in progress',
  delivery_unknown: 'Delivery status unknown',
  pdf_missing: 'The PDF was not ready in time',
  send_failed: 'Sending failed',
}

export const skipReason = (value: unknown): string | undefined =>
  typeof value === 'string' ? (EMAIL_SKIP_REASONS[value] ?? value.replace(/_/g, ' ')) : undefined
