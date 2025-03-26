import { InviteOnboardingForm } from '@/features/onboarding/inviteOnboardingForm'

export const InviteOnboarding = () => {
  return (
    <>
      <div className="flex-1 bg-[#111] rounded-l-lg">
        <InviteOnboardingForm />
      </div>
      <div className="flex-1 bg-[#313131] rounded-r-lg pl-32">
        <img src="/img/onboarding/invite.svg" alt="user onboarding" className="h-full" />
      </div>
    </>
  )
}
