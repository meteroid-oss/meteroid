import { Button, Separator, Textarea, cn } from '@md/ui'
import { ExternalLink, Heart, MessageCircle, MessageSquareIcon, SendHorizonalIcon } from 'lucide-react'
import { FunctionComponent, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'

import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { copyToClipboard } from '@/lib/helpers'
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
      body: JSON.stringify({ feedback, user, context }),
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
                <span className="flex items-center gap-1 ml-2 text-sm text-muted-foreground">
                  We would <Heart size="12" fill="red" strokeWidth={0} className="" /> your feedback !
                </span>
              </div>
            </div>
            <p className="text-sm text-muted-foreground mb-4">
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
          </div>
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
