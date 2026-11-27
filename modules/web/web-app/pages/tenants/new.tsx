import { Button, Card, Form } from '@md/ui'
import { ChevronLeft } from 'lucide-react'
import { FunctionComponent } from 'react'
import { useNavigate } from 'react-router'

import { CreateTenantFields, useCreateTenantForm } from '@/features/tenants/CreateTenant'

/**
 * Where an organization without tenants lands (see OrganizationRoot): with no tenant, there is
 * no layout to open the new-tenant modal over. Elsewhere tenants are created in that modal.
 */
export const TenantNew: FunctionComponent = () => {
  const navigate = useNavigate()
  const { methods, submit, error, isPending } = useCreateTenantForm()

  return (
    <main className="flex  flex-col flex-1 w-full max-w-screen-2xl pl-8 pr-2 mx-auto h-full overflow-x-hidden ">
      <div className="pt-4">
        <div className="flex space-x-4 items-center">
          <Button variant="ghost" onClick={() => navigate(-1)} className="">
            <ChevronLeft size={16} />
          </Button>

          <div className="text-lg font-semibold">New tenant</div>
        </div>
      </div>

      <div className="relative py-8 px-4 max-w-screen-sm">
        <div className="  space-y-6 w-full h-full overflow-x-hidden">
          <Form {...methods}>
            <form onSubmit={submit}>
              <Card className="px-8 py-6">
                <div className="max-w-xs">
                  <CreateTenantFields methods={methods} />
                </div>
                {error && <div className="text-destructive text-xs pt-2">{error}</div>}

                <div className="w-full flex justify-end items-center pt-6">
                  <Button
                    variant="ghost"
                    className="!rounded-r-none"
                    size="sm"
                    type="button"
                    onClick={() => navigate(-1)}
                  >
                    Cancel
                  </Button>
                  <Button
                    className="!rounded-l-none"
                    size="sm"
                    disabled={!methods.formState.isValid || isPending}
                  >
                    Save
                  </Button>
                </div>
              </Card>
            </form>
          </Form>
        </div>
      </div>
    </main>
  )
}
