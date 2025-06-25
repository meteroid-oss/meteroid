import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  ComboboxFormField,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  InputFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { z } from 'zod'

import { CurrencySelect } from '@/components/CurrencySelect'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  createBankAccount,
  listBankAccounts,
} from '@/rpc/api/bankaccounts/v1/bankaccounts-BankAccountsService_connectquery'
import {
  AccountNumberBicSwift,
  AccountNumberRoutingNumber,
  BankAccountData,
  IbanBicSwift,
  SortCodeAccountNumber,
} from '@/rpc/api/bankaccounts/v1/models_pb'
import { getCountries } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

const baseSchema = z.object({
  currency: z.string().min(3, 'Currency is required'),
  country: z.string().min(2, 'Country is required'),
  bankName: z.string().min(1, 'Bank name is required'),
  format: z.enum(['iban', 'account_bic', 'account_routing', 'sort_code']),
})

const ibanSchema = baseSchema.extend({
  format: z.literal('iban'),
  iban: z.string().min(1, 'IBAN is required'),
  bicSwift: z.string().optional(),
})

const accountBicSchema = baseSchema.extend({
  format: z.literal('account_bic'),
  accountNumber: z.string().min(1, 'Account number is required'),
  bicSwift: z.string().optional(),
})

const accountRoutingSchema = baseSchema.extend({
  format: z.literal('account_routing'),
  accountNumber: z.string().min(1, 'Account number is required'),
  routingNumber: z.string().min(1, 'Routing number is required'),
})

const sortCodeSchema = baseSchema.extend({
  format: z.literal('sort_code'),
  accountNumber: z.string().min(1, 'Account number is required'),
  sortCode: z.string().min(1, 'Sort code is required'),
})

const bankAccountSchema = z.discriminatedUnion('format', [
  ibanSchema,
  accountBicSchema,
  accountRoutingSchema,
  sortCodeSchema,
])

type BankAccountFormData = z.infer<typeof bankAccountSchema>

interface AddBankAccountModalProps {
  open: boolean
  onClose: () => void
}

export const AddBankAccountModal = ({ open, onClose }: AddBankAccountModalProps) => {
  const queryClient = useQueryClient()
  const getCountriesQuery = useQuery(getCountries)

  const createBankAccountMut = useMutation(createBankAccount, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listBankAccounts),
      })
      toast.success('Bank account created successfully')
      onClose()
      methods.reset()
    },
    onError: error => {
      toast.error('Failed to create bank account')
      console.error(error)
    },
  })

  const methods = useZodForm({
    schema: bankAccountSchema,
    defaultValues: {
      currency: '',
      country: '',
      bankName: '',
      format: 'iban' as const,
    },
  })

  const onSubmit = async (data: BankAccountFormData) => {
    const bankAccountData = new BankAccountData({
      currency: data.currency,
      country: data.country,
      bankName: data.bankName,
    })

    switch (data.format) {
      case 'iban':
        bankAccountData.format = {
          case: 'ibanBicSwift',
          value: new IbanBicSwift({
            iban: data.iban,
            bicSwift: data.bicSwift,
          }),
        }
        break
      case 'account_bic':
        bankAccountData.format = {
          case: 'accountNumberBicSwift',
          value: new AccountNumberBicSwift({
            accountNumber: data.accountNumber,
            bicSwift: data.bicSwift,
          }),
        }
        break
      case 'account_routing':
        bankAccountData.format = {
          case: 'accountNumberRoutingNumber',
          value: new AccountNumberRoutingNumber({
            accountNumber: data.accountNumber,
            routingNumber: data.routingNumber,
          }),
        }
        break
      case 'sort_code':
        bankAccountData.format = {
          case: 'sortCodeAccountNumber',
          value: new SortCodeAccountNumber({
            accountNumber: data.accountNumber,
            sortCode: data.sortCode,
          }),
        }
        break
    }

    return await createBankAccountMut.mutateAsync({ data: bankAccountData })
  }

  const formatWatch = methods.watch('format')

  return (
    <Dialog open={open} onOpenChange={onClose}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>Add Bank Account</DialogTitle>
          <DialogDescription>
            Add a new bank account to receive payments via bank transfer
          </DialogDescription>
        </DialogHeader>

        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <CurrencySelect
                name="currency"
                label="Currency"
                placeholder="USD"
                layout="vertical"
                control={methods.control}
                className="uppercase"
              />

              <ComboboxFormField
                name="country"
                label="Country"
                control={methods.control}
                className="rounded-b-none   text-xs"
                placeholder="Country"
                hasSearch
                options={
                  getCountriesQuery.data?.countries.map(country => ({
                    label: (
                      <span className="flex flex-row">
                        <span className="pr-2">{getCountryFlagEmoji(country.code)}</span>
                        <span>{country.name}</span>
                      </span>
                    ),
                    value: country.code,
                    keywords: [country.name, country.code],
                  })) ?? []
                }
              />
            </div>

            <InputFormField
              name="bankName"
              label="Bank Name"
              placeholder="Bank of America"
              control={methods.control}
            />

            <SelectFormField
              name="format"
              label="Account Format"
              placeholder="Select account format"
              control={methods.control}
            >
              <SelectItem value="iban">IBAN</SelectItem>
              <SelectItem value="account_bic">Account Number + BIC/SWIFT</SelectItem>
              <SelectItem value="account_routing">Account + Routing Number (US)</SelectItem>
              <SelectItem value="sort_code">Sort Code + Account (UK)</SelectItem>
            </SelectFormField>

            {formatWatch === 'iban' && (
              <>
                <InputFormField
                  name="iban"
                  label="IBAN"
                  placeholder="DE89 3704 0044 0532 0130 00"
                  control={methods.control}
                />
                <div className="grid grid-cols-2 gap-4">
                  <InputFormField
                    name="bicSwift"
                    label="BIC / SWIFT (Optional)"
                    placeholder="COBADEFF"
                    control={methods.control}
                  />
                </div>
              </>
            )}

            {formatWatch === 'account_bic' && (
              <>
                <InputFormField
                  name="accountNumber"
                  label="Account Number"
                  placeholder="123456789"
                  control={methods.control}
                />
                <div className="grid grid-cols-2 gap-4">
                  <InputFormField
                    name="bicSwift"
                    label="BIC / SWIFT (Optional)"
                    placeholder="COBADEFF"
                    control={methods.control}
                  />
                </div>
              </>
            )}

            {formatWatch === 'account_routing' && (
              <div className="space-y-4">
                <InputFormField
                  name="accountNumber"
                  label="Account Number"
                  placeholder="123456789"
                  control={methods.control}
                />
                <InputFormField
                  name="routingNumber"
                  label="Routing Number"
                  placeholder="026009593"
                  control={methods.control}
                />
              </div>
            )}

            {formatWatch === 'sort_code' && (
              <div className="space-y-4">
                <InputFormField
                  name="sortCode"
                  label="Sort Code"
                  placeholder="12-34-56"
                  control={methods.control}
                />
                <InputFormField
                  name="accountNumber"
                  label="Account Number"
                  placeholder="12345678"
                  control={methods.control}
                />
              </div>
            )}

            <DialogFooter>
              <Button variant="outline" type="button" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" disabled={createBankAccountMut.isPending}>
                {createBankAccountMut.isPending ? 'Creating...' : 'Create Bank Account'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
