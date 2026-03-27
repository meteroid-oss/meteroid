import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@md/ui'
import { Link2 } from 'lucide-react'
import { FunctionComponent  } from 'react'

export const PlatformSettingsTab: FunctionComponent = () => {


    return (
      <div className="py-4">
        <Card className="max-w-2xl">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Link2 size={20} />
              Connect Platform (Enterprise / Cloud)
            </CardTitle>
            <CardDescription>
              Connect lets you manage billing on behalf of other organizations. Create connected
              accounts, onboard them via OAuth or express flow, and handle their subscriptions,
              invoices and payments through your platform.
            </CardDescription>
          </CardHeader>
        </Card>
      </div>
    )

}
