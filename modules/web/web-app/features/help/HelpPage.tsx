import {
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Separator,
  Textarea,
  cn,
} from '@md/ui'
import {
  ChevronDown,
  ExternalLink,
  Heart,
  MessageCircle,
  MessageSquareIcon,
  SendHorizonalIcon,
} from 'lucide-react'
import { FunctionComponent, useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { toast } from 'sonner'

import { LocalId } from '@/components/LocalId'
import { slotItems } from '@/edition'
import { IdConverter } from '@/features/help/IdConverter'
import { useIsCloud } from '@/hooks/useIsCloud'
import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { copyToClipboard } from '@/lib/helpers'
import { getCurrentOrganizations } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

const copyEmail = () => {
  copyToClipboard('team@meteroid.com')
  toast.success('Email copied to clipboard')
}

const sendFeedback = async (
  feedback: string,
  user: { email: string } | undefined,
  cb: () => void
) => {
  const context = {
    pathName: window.location.pathname,
    isLocalhost: window.location.hostname === 'localhost',
    env: process.env.NODE_ENV,
  }

  try {
    const res = await fetch('https://metero.id/ossapi/feedback', {
      method: 'POST',
      body: JSON.stringify({ feedback, user: user && { email: user.email }, context }),
    })

    if (res.status === 200) {
      toast.success('Sent ! Thanks for your feedback 💚')
      cb()
    } else {
      toast.error('Failed to send feedback, please send us an email at hey@meteroid.com')
    }
  } catch (e) {
    toast.error('Failed to send feedback, please send us an email at hey@meteroid.com')
  }
}

export const HelpPage: FunctionComponent = () => {
  const [feedback, setFeedback] = useState('')
  const [messageVisible, setMessageVisible] = useState(false)
  const hideTimerRef = useRef<NodeJS.Timeout | number | null>(null)
  const user = useQuery(me)
  const isCloud = useIsCloud()

  const { tenant } = useTenant()
  const organization = useQuery(getCurrentOrganizations).data?.organization
  const [searchParams] = useSearchParams()
  const topicId = searchParams.get('topic')
  const topic = slotItems('help.topics').find(item => item.id === topicId)
  const prefill = useCallback((text: string) => setFeedback(current => current || text), [])

  const isLocalhost = ['localhost', '127.0.0.1'].includes(window.location.hostname)
  const isMeteroidUser = user.data?.user?.email?.endsWith('@meteroid.com') ?? false
  const showIdConverter = isLocalhost || isMeteroidUser
  // The IDs are for Meteroid's support, which only Cloud organizations reach.
  const showTechnicalContext = isCloud === true || isMeteroidUser

  const showMessage = () => {
    if (hideTimerRef.current) {
      clearTimeout(hideTimerRef.current)
    }
    setMessageVisible(true)
  }

  const hideMessage = () => {
    if (hideTimerRef.current) {
      clearTimeout(hideTimerRef.current)
    }
    hideTimerRef.current = setTimeout(() => {
      setMessageVisible(false)
    }, 10000)
  }

  useEffect(() => {
    // Show the message after 5 seconds for a nice touch
    const showTimer = setTimeout(() => {
      showMessage()
      hideMessage()
    }, 5000)

    return () => {
      clearTimeout(showTimer)
      hideTimerRef.current !== null && clearTimeout(hideTimerRef.current)
    }
  }, [])

  return (
    <div className="space-y-6 w-full border-t pt-6">
      <div>
        <h1 className="text-2xl font-semibold pb-2">Help & Feedback</h1>
        <p className="text-sm text-muted-foreground">
          Get help, share feedback, or connect with the Meteroid community
        </p>
      </div>

      {topic && <topic.Notice key={topic.id} prefill={prefill} />}

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Feedback Section */}
        <div className="space-y-4">
          <div className="border border-border rounded-lg p-6">
            <div
              className="flex items-center gap-2 mb-4"
              onMouseEnter={showMessage}
              onMouseLeave={hideMessage}
            >
              <MessageSquareIcon size={20} strokeWidth={1.5} />
              <h2 className="text-lg font-semibold">Quick Feedback</h2>
              <div
                className={cn(
                  'transition-all ease-out duration-1000 max-w-0 overflow-hidden whitespace-nowrap inline-block',
                  messageVisible ? 'max-w-[230px]' : ''
                )}
              >
                <span className="flex items-center gap-1 ml-2 text-xs text-muted-foreground">
                  We would <Heart size="12" fill="red" strokeWidth={0} className="" /> your feedback !
                </span>
              </div>
            </div>
            <p className="text-xs text-muted-foreground mb-4">
              <span>Meteroid is built with the community.</span>
              <br />
              <span>Let us know what you need!</span>
            </p>

            <div className="flex flex-row gap-2 align-bottom">
              <Textarea
                placeholder="I'd love to see..."
                value={feedback}
                className={cn('focus:min-h-40', !!feedback.length && 'min-h-40')}
                maxLength={1000}
                onChange={e => setFeedback(e.target.value)}
              />
              <div className="flex flex-col gap-2 justify-between items-center">
                <span className="text-xs text-muted-foreground">{feedback.length}/1000</span>
                <Button
                  variant="secondary"
                  size="sm"
                  className="self-end"
                  disabled={!feedback.length}
                  onClick={() => sendFeedback(feedback, user.data?.user, () => setFeedback(''))}
                >
                  <SendHorizonalIcon size="12" />
                </Button>
              </div>
            </div>
            <div className="text-xs pt-2">
              or{' '}
              <a
                className="underline"
                href="mailto:team@meteroid.com"
                target="_blank"
                rel="noopener noreferrer"
                onClick={() => copyEmail()}
              >
                email us
              </a>
            </div>
            <p className="pt-2 text-[11px] leading-snug text-muted-foreground">
              Sending shares your message, your email address and the current URL path with Meteroid,
              so we can get back to you.
            </p>
          </div>

          {showTechnicalContext && (
            <Collapsible className="border border-border rounded-lg group">
              <CollapsibleTrigger className="flex items-center justify-between w-full p-6 text-left">
                <h2 className="text-sm font-semibold">Technical context</h2>
                <ChevronDown
                  size={16}
                  className="text-muted-foreground transition-transform group-data-[state=open]:rotate-180"
                />
              </CollapsibleTrigger>
              <CollapsibleContent className="px-6 pb-6">
                <p className="text-xs text-muted-foreground mb-4">
                  Share these details when reaching out so we can help you faster.
                </p>

                <div className="space-y-2 text-xs">
                  <div className="flex items-center justify-between gap-4">
                    <span className="text-muted-foreground">Organization ID</span>
                    {organization ? (
                      <LocalId localId={organization.id} className="max-w-[200px]" />
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </div>
                  <div className="flex items-center justify-between gap-4">
                    <span className="text-muted-foreground">Tenant ID</span>
                    {tenant ? (
                      <LocalId localId={tenant.id} className="max-w-[200px]" />
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </div>
                </div>

                {showIdConverter && (
                  <div className="mt-4 pt-4 border-t border-border">
                    <h3 className="text-xs font-semibold mb-2">ID converter</h3>
                    <IdConverter />
                  </div>
                )}
              </CollapsibleContent>
            </Collapsible>
          )}
        </div>

        {/* Resources Section */}
        <div className="space-y-4">
          <div className="border border-border rounded-lg p-6">
            <div className="flex items-center gap-2 mb-4">
              <Heart size={20} strokeWidth={1.5} />
              <h2 className="text-lg font-semibold">Resources</h2>
            </div>

            <div className="space-y-3">
              <a
                href="https://docs.meteroid.com"
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-center justify-between p-3 border border-border rounded-lg hover:bg-accent transition-colors"
              >
                <div className="flex items-center gap-3">
                  <ExternalLink size={16} className="text-muted-foreground" />
                  <div>
                    <div className="font-medium">Documentation</div>
                    <div className="text-xs text-muted-foreground">
                      Comprehensive guides and API reference
                    </div>
                  </div>
                </div>
                <ExternalLink size={14} className="text-muted-foreground" />
              </a>


              <a
                href={`${env.meteroidRestApiUri}/scalar`}
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-center justify-between p-3 border border-border rounded-lg hover:bg-accent transition-colors"
              >
                <div className="flex items-center gap-3">
                  <ExternalLink size={16} className="text-muted-foreground" />
                  <div>
                    <div className="font-medium">API Playground</div>
                    <div className="text-xs text-muted-foreground">
                      Interactive API exploration and testing
                    </div>
                  </div>
                </div>
                <ExternalLink size={14} className="text-muted-foreground" />
              </a>

              <Separator />


              <div className="space-y-2">
                <h3 className="text-sm font-semibold">Community</h3>
                <div className="space-y-2">
                  <div
                    className="relative space-y-2 overflow-hidden rounded px-5 py-4 pb-12 shadow-md h-[100px]"
                    style={{ background: '#404EED' }}
                  >
                    <a
                      href="https://go.meteroid.com/discord"
                      target="_blank"
                      className="dark block cursor-pointer"
                      rel="noreferrer"
                    >
                      <img
                        className="absolute left-0 top-0 opacity-50"
                        src="/img/support/discord-bg-small.jpg"
                        style={{ objectFit: 'cover' }}
                        alt="discord illustration header"
                      />

                      <Button
                        hasIcon
                        className="absolute left-3 top-3 opacity-80 bg-foreground"
                      >
                        <span style={{ color: '#404EED' }}>Join us on Discord !</span>
                      </Button>
                    </a>
                  </div>

                  <div className="relative space-y-2 overflow-hidden rounded px-5 py-4 pb-12 shadow-md h-[100px]">
                    <a
                      href="https://github.com/meteroid-oss/meteroid/discussions"
                      target="_blank"
                      className="block cursor-pointer"
                      rel="noreferrer"
                    >
                      <img
                        className="absolute left-0 top-0 opacity-50"
                        src="/img/support/github-bg.png?v-1"
                        style={{
                          objectFit: 'cover',
                        }}
                        alt="github discussions header"
                      />
                      <Button
                        variant="secondary"
                        hasIcon
                        className="absolute left-3 top-3 opacity-80 dark:bg-secondary bg-foreground dark:text-secondary-foreground text-secondary"
                      >
                        <MessageCircle size={14} /> GitHub Discussions
                      </Button>
                    </a>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
