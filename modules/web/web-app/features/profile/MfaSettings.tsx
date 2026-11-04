import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Form,
  InputFormField,
  Label,
  Modal,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ChevronDownIcon, ShieldCheckIcon } from 'lucide-react'
import { QRCodeSVG } from 'qrcode.react'
import { useState } from 'react'
import { toast } from 'sonner'
import { z } from 'zod'

import {
  SettingsRow,
  SettingsRowGroup,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useAutoSubmitOtp } from '@/hooks/useAutoSubmitOtp'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  confirmMfaEnrollment,
  disableMfa,
  initMfaEnrollment,
  me,
  regenerateMfaRecoveryCodes,
} from '@/rpc/api/users/v1/users-UsersService_connectquery'

type Stage =
  | { kind: 'idle' }
  | { kind: 'enrolling'; secretBase32: string; otpauthUri: string }
  | { kind: 'codes'; recoveryCodes: string[] }

const codeSchema = z.object({
  code: z.string().regex(/^\d{6}$/, 'Enter the 6-digit code from your authenticator.'),
})

const passwordSchema = z.object({
  password: z.string().min(1, 'Password is required.'),
})

export const MfaSettings = () => {
  const meQuery = useQuery(me)
  const queryClient = useQueryClient()
  const [stage, setStage] = useState<Stage>({ kind: 'idle' })
  const [actionModal, setActionModal] = useState<'regen' | 'disable' | null>(null)

  const mfaEnabled = meQuery.data?.user?.mfaEnabled ?? false
  const enrollmentRequired = meQuery.data?.mfaEnrollmentRequired ?? false

  const initMut = useMutation(initMfaEnrollment, {
    onSuccess: data => {
      setStage({
        kind: 'enrolling',
        secretBase32: data.secretBase32,
        otpauthUri: data.otpauthUri,
      })
    },
  })

  const confirmMethods = useZodForm({
    schema: codeSchema,
    defaultValues: { code: '' },
    mode: 'onSubmit',
  })

  const confirmMut = useMutation(confirmMfaEnrollment, {
    onSuccess: data => {
      setStage({ kind: 'codes', recoveryCodes: data.recoveryCodes })
      confirmMethods.reset()
      toast.success('Two-factor authentication enabled.')
    },
  })

  const submitConfirm = confirmMethods.handleSubmit(data =>
    confirmMut
      .mutateAsync({ code: data.code })
      .catch(err => toast.error(err?.rawMessage ?? 'Invalid code, try again.')),
  )

  useAutoSubmitOtp(confirmMethods.watch('code'), confirmMut.isPending, () => void submitConfirm())

  const disableMethods = useZodForm({
    schema: passwordSchema,
    defaultValues: { password: '' },
    mode: 'onSubmit',
  })

  const disableMut = useMutation(disableMfa, {
    onSuccess: () => {
      disableMethods.reset()
      setActionModal(null)
      void queryClient.invalidateQueries({ queryKey: createConnectQueryKey({ schema: me, cardinality: 'finite' }) })
      toast.success('Two-factor authentication disabled.')
    },
  })

  const regenMethods = useZodForm({
    schema: passwordSchema,
    defaultValues: { password: '' },
    mode: 'onSubmit',
  })

  const regenMut = useMutation(regenerateMfaRecoveryCodes, {
    onSuccess: data => {
      regenMethods.reset()
      setActionModal(null)
      setStage({ kind: 'codes', recoveryCodes: data.recoveryCodes })
      toast.success('Recovery codes regenerated.')
    },
  })

  if (stage.kind === 'enrolling') {
    return (
      <SettingsSection
        divided={false}
        title="Set up your authenticator"
        description="Scan this QR code with your authenticator app (1Password, Authy, Google Authenticator, etc.), or enter the secret manually. Then enter the 6-digit code your app displays to confirm."
        className="max-w-3xl"
      >
        <div className="flex flex-col items-center gap-3 sm:flex-row sm:items-start">
          <div className="rounded bg-white p-3">
            <QRCodeSVG value={stage.otpauthUri} size={176} level="M" />
          </div>
          <div className="flex-1 space-y-2">
            <Label>Account secret (base32)</Label>
            <pre className="rounded bg-muted px-3 py-2 font-mono text-sm break-all">
              {stage.secretBase32}
            </pre>
            <details className="text-xs text-muted-foreground">
              <summary className="cursor-pointer">Show otpauth URL</summary>
              <pre className="mt-2 rounded bg-muted px-3 py-2 font-mono text-xs whitespace-pre-wrap break-all">
                {stage.otpauthUri}
              </pre>
            </details>
          </div>
        </div>
        <Form {...confirmMethods}>
          <form onSubmit={submitConfirm} className="space-y-3">
            <InputFormField
              name="code"
              label="Verification code"
              control={confirmMethods.control}
              placeholder="123456"
              autoComplete="one-time-code"
              autoFocus
            />
            <div className="flex gap-2">
              <Button type="submit" variant="primary" disabled={confirmMut.isPending}>
                Verify and enable
              </Button>
              <Button type="button" variant="secondary" onClick={() => setStage({ kind: 'idle' })}>
                Cancel
              </Button>
            </div>
          </form>
        </Form>
      </SettingsSection>
    )
  }

  if (stage.kind === 'codes') {
    return (
      <SettingsSection
        divided={false}
        title="Save your recovery codes"
        description="Each code can be used once if you lose access to your authenticator. Store them somewhere safe — we cannot show them again."
        className="max-w-3xl"
      >
        <pre className="rounded-lg border border-border bg-muted px-4 py-3 font-mono text-sm leading-7">
          {stage.recoveryCodes.join('\n')}
        </pre>
        <Button
          className="mt-4"
          onClick={() => {
            void queryClient.invalidateQueries({ queryKey: createConnectQueryKey({ schema: me, cardinality: 'finite' }) })
            setStage({ kind: 'idle' })
          }}
          variant="primary"
        >
          I&apos;ve saved them
        </Button>
      </SettingsSection>
    )
  }

  // idle
  return (
    <SettingsSection
      divided={false}
      title="Two-factor authentication"
      description="Add an extra layer of security to your account by requiring a one-time code at sign-in."
      className="max-w-3xl"
    >
      <SettingsRowGroup>
        <SettingsRow
          title={
            <span className="flex items-center gap-2">
              Authenticator app
              {mfaEnabled && (
                <Badge variant="success" size="sm" className="gap-1">
                  <ShieldCheckIcon size={12} />
                  Enabled
                </Badge>
              )}
            </span>
          }
          description={
            mfaEnabled
              ? 'You will be asked for a code each time you sign in.'
              : enrollmentRequired
                ? 'Your organization requires two-factor authentication. Please enroll to continue.'
                : 'Use an app like 1Password, Authy or Google Authenticator.'
          }
        >
          {mfaEnabled ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" size="sm" hasIcon>
                  Actions
                  <ChevronDownIcon size={16} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => setActionModal('regen')}>
                  Regenerate recovery codes
                </DropdownMenuItem>
                <DropdownMenuItem
                  className="text-destructive focus:text-destructive"
                  onClick={() => setActionModal('disable')}
                >
                  Disable two-factor authentication
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : (
            <Button
              variant="primary"
              size="sm"
              disabled={initMut.isPending}
              onClick={() =>
                initMut.mutateAsync({}).catch(err =>
                  toast.error(err?.rawMessage ?? 'Could not start enrollment.'),
                )
              }
            >
              Enable
            </Button>
          )}
        </SettingsRow>
      </SettingsRowGroup>

      <Modal
        visible={actionModal === 'regen'}
        size="small"
        header="Regenerate recovery codes"
        confirmText="Regenerate"
        loading={regenMut.isPending}
        onCancel={() => {
          regenMethods.reset()
          setActionModal(null)
        }}
        onConfirm={() =>
          regenMethods.handleSubmit(data =>
            regenMut.mutateAsync({ password: data.password }).catch(err =>
              toast.error(err?.rawMessage ?? 'Invalid password.'),
            ),
          )()
        }
      >
        <Form {...regenMethods}>
          <form
            className="px-5 py-4 space-y-3"
            onSubmit={regenMethods.handleSubmit(data =>
              regenMut.mutateAsync({ password: data.password }).catch(err =>
                toast.error(err?.rawMessage ?? 'Invalid password.'),
              ),
            )}
          >
            <p className="text-sm text-muted-foreground">
              Invalidates your existing codes and issues a fresh set. Confirm your password to
              continue.
            </p>
            <InputFormField
              name="password"
              label="Confirm password"
              control={regenMethods.control}
              type="password"
              placeholder="••••••••"
            />
          </form>
        </Form>
      </Modal>

      <Modal
        visible={actionModal === 'disable'}
        size="small"
        header="Disable two-factor authentication"
        confirmText="Disable"
        loading={disableMut.isPending}
        onCancel={() => {
          disableMethods.reset()
          setActionModal(null)
        }}
        onConfirm={() =>
          disableMethods.handleSubmit(data =>
            disableMut.mutateAsync({ password: data.password }).catch(err =>
              toast.error(err?.rawMessage ?? 'Invalid password.'),
            ),
          )()
        }
      >
        <Form {...disableMethods}>
          <form
            className="px-5 py-4 space-y-3"
            onSubmit={disableMethods.handleSubmit(data =>
              disableMut.mutateAsync({ password: data.password }).catch(err =>
                toast.error(err?.rawMessage ?? 'Invalid password.'),
              ),
            )}
          >
            <p className="text-sm text-muted-foreground">
              Your account will no longer be protected by two-factor authentication. Confirm your
              password to continue.
            </p>
            <InputFormField
              name="password"
              label="Confirm password"
              control={disableMethods.control}
              type="password"
              placeholder="••••••••"
            />
          </form>
        </Form>
      </Modal>
    </SettingsSection>
  )
}
