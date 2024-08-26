import { useMutation } from '@connectrpc/connect-query'
import { Button, Card, ComboboxFormField, Form, InputFormField } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { Loading } from '@/components/Loading'
import { useOrganizationSlug } from '@/hooks/useOrganization'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { getCurrencies } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import {
  activeTenant,
  updateTenant,
} from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

const generalSchema = z.object({
  tradeName: z.string().min(1, 'Trade name is required'),
  tenantName: z.string().min(1, 'Tenant name is required'),
  slug: z.string().min(1, 'Tenant name is required'),
  reportingCurrency: z.string().min(1, 'Tenant name is required'),
})

export const GeneralTab = () => {
  const activeTenantQuery = useQuery(activeTenant)
  const activeTenantData = activeTenantQuery.data

  const getCurrenciesQuery = useQuery(getCurrencies)
  const currencies = getCurrenciesQuery.data?.currencies ?? []

  const queryClient = useQueryClient()
  const navigate = useNavigate()

  const organizationSlug = useOrganizationSlug()

  const updateTenantMut = useMutation(updateTenant, {
    onSuccess: async res => {
      if (res.tenant) {
        const newSlug = res.tenant.slug
        const hasChanged = newSlug !== activeTenantData?.tenant?.slug

        await queryClient.invalidateQueries()
        toast.success('Updated successfully !')

        if (hasChanged) {
          navigate(`/${organizationSlug}/${newSlug}/settings`, { replace: true })
        }
      }
    },
    onError: error => {
      toast.error(error.rawMessage)
    },
  })

  const methods = useZodForm({
    schema: generalSchema,
    defaultValues: {
      tradeName: '',
      tenantName: '',
      slug: '',
      reportingCurrency: '',
    },
    mode: 'onSubmit',
  })

  useEffect(() => {
    if (activeTenantData?.tenant && currencies.length > 0) {
      methods.reset({
        tradeName: activeTenantData.tradeName,
        tenantName: activeTenantData.tenant.name,
        slug: activeTenantData.tenant.slug,
        reportingCurrency: activeTenantData.tenant.reportingCurrency,
      })
    }
  }, [activeTenantData, currencies])

  if (activeTenantQuery.isLoading || getCurrenciesQuery.isLoading) {
    return <Loading />
  }

  const onSubmit = async (values: z.infer<typeof generalSchema>) => {
    await updateTenantMut.mutateAsync({
      data: {
        tradeName: values.tradeName,
        slug: values.slug,
        reportingCurrency: values.reportingCurrency,
        name: values.tenantName,
      },
    })
  }

  return (
    <div className="flex flex-col gap-4">
      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
          <Card className="px-8 py-6 max-w-[950px]  space-y-4">
            <div className="grid grid-cols-6 gap-4  ">
              <div className="col-span-3">
                <h3 className="font-medium text-lg">Informations</h3>
              </div>
            </div>
            <div className="grid grid-cols-6 gap-4 pt-1 ">
              <InputFormField
                name="tradeName"
                label="Company trade name"
                control={methods.control}
                placeholder="ACME"
                containerClassName="col-span-6"
              />

              <InputFormField
                name="tenantName"
                control={methods.control}
                label="Tenant name"
                placeholder="Production"
                containerClassName="col-span-3"
              />
              <InputFormField
                name="slug"
                control={methods.control}
                label="Tenant slug"
                placeholder="prod"
                containerClassName="col-span-3"
              />

              <ComboboxFormField
                name="reportingCurrency"
                label="Reporting currency"
                control={methods.control}
                placeholder="Select"
                hasSearch
                containerClassName="col-span-3"
                description="The currency used for reporting and dashboards"
                options={
                  currencies.map(currency => ({
                    label: (
                      <span className="flex flex-row">
                        <span className="pr-2">{currency.name}</span>
                        <span>({currency.code})</span>
                      </span>
                    ),
                    value: currency.code,
                    keywords: [currency.name, currency.code],
                  })) ?? []
                }
              />
            </div>

            <div className="pt-10 flex justify-end items-center ">
              <div>
                <Button
                  size="sm"
                  disabled={
                    //   !methods.formState.isValid ||
                    !methods.formState.isDirty || updateTenantMut.isPending
                  }
                >
                  Save changes
                </Button>
              </div>
            </div>
          </Card>
        </form>
      </Form>
    </div>
  )
}
