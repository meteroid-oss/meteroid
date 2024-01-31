import { spaces } from '@md/foundation'
import {
  Button,
  Flex,
  FormItem,
  Input,
  Select,
  SelectItem,
  Toggle,
  Wizard,
  useWizard,
} from '@md/ui'
import { useState, type FunctionComponent } from 'react'
import { z } from 'zod'

import StepTitle from '@/features/onboarding/components/StepTitle'
import WizardWrapper from '@/features/onboarding/components/WizardWrapper/WizardWrapper'
import { useZodForm } from '@/hooks/useZodForm'

export const Step3: FunctionComponent = () => {
  const { previousStep, nextStep } = useWizard()
  const [selectedIndustries, setSelectedIndustries] = useState<string[]>([])
  const [selectedSizes, setSelectedSizes] = useState<string[]>([])
  const [selectedRole, setSelectedRole] = useState<string>('')
  const [selectedReference, setSelectedReference] = useState<string>('')
  const [otherIndustry, setOtherIndustry] = useState('')

  const detailsForm = z.object({
    industries: z
      .array(z.string())
      .refine(value => value.length > 0, 'Please select at least one industry'),
    size: z.array(z.string()),
    role: z.string().nonempty('Please select a role'),
    reference: z.string().nonempty('Please select a reference'),
  })

  const methods = useZodForm({
    schema: detailsForm,
    defaultValues: {
      industries: selectedIndustries,
      size: selectedSizes,
      role: selectedRole,
      reference: selectedReference,
    },
  })

  return (
    <Wizard.AnimatedStep previousStep={previousStep}>
      <WizardWrapper>
        <StepTitle>Tell us a bit about yourself</StepTitle>
        <Flex direction="column" gap={spaces.space9}>
          <Flex direction="column" gap={spaces.space6}>
            <FormItem
              name="industries"
              label="What industry are you in?"
              error={methods.formState.errors.industries?.message}
            >
              <Flex direction="column" gap={spaces.space4}>
                <Flex direction="row" gap={spaces.space4} wrap="wrap">
                  {IndustriesList.map((industry, industryIndex) => (
                    <Toggle
                      key={industryIndex}
                      onPressedChange={state => {
                        if (state) {
                          setSelectedIndustries(prevSelectedIndustries => [
                            ...prevSelectedIndustries,
                            industry,
                          ])
                        } else {
                          setSelectedIndustries(prevSelectedIndustries =>
                            prevSelectedIndustries.filter(
                              selectedIndustry => selectedIndustry !== industry
                            )
                          )
                        }
                        !selectedIndustries.includes('Other') && setOtherIndustry('') // reset other industry input
                      }}
                    >
                      {industry}
                    </Toggle>
                  ))}
                </Flex>
                {selectedIndustries.includes('Other') && (
                  <Input
                    type="text"
                    placeholder="Other Industry.."
                    value={otherIndustry}
                    onChange={event => setOtherIndustry(event.target.value)}
                  />
                )}
              </Flex>
            </FormItem>
            <FormItem
              name="size"
              label="What is the size of your company?"
              error={methods.formState.errors.size?.message}
            >
              <Flex direction="row" gap={spaces.space4} wrap="wrap">
                {SizeList.map((size, sizeIndex) => (
                  <Toggle
                    key={sizeIndex}
                    onPressedChange={state => {
                      if (state) {
                        setSelectedSizes(prevSelectedSizes => [...prevSelectedSizes, size])
                      } else {
                        setSelectedSizes(prevSelectedSizes =>
                          prevSelectedSizes.filter(selectedSize => selectedSize !== size)
                        )
                      }
                    }}
                  >
                    {size}
                  </Toggle>
                ))}
              </Flex>
            </FormItem>
            <FormItem
              name="role"
              label="Which role most closely fits your skillset?"
              error={methods.formState.errors.role?.message}
            >
              <Select placeholder="Select role" onValueChange={value => setSelectedRole(value)}>
                {RoleList.map((role, roleIndex) => (
                  <SelectItem key={roleIndex} value={role}>
                    {role}
                  </SelectItem>
                ))}
              </Select>
            </FormItem>
            <FormItem
              name="reference"
              label="How did you hear about Meteroid?"
              error={methods.formState.errors.reference?.message}
            >
              <Select
                placeholder="Select reference"
                onValueChange={value => setSelectedReference(value)}
              >
                {ReferenceList.map((reference, referenceIndex) => (
                  <SelectItem key={referenceIndex} value={reference}>
                    {reference}
                  </SelectItem>
                ))}
              </Select>
            </FormItem>
          </Flex>
          <Flex direction="row" align="center" justify="space-between">
            <Button variant="tertiary" onClick={previousStep}>
              Back
            </Button>
            <Button variant="primary" onClick={nextStep}>
              Next
            </Button>
          </Flex>
        </Flex>
      </WizardWrapper>
    </Wizard.AnimatedStep>
  )
}

const IndustriesList = [
  'SaaS',
  'E-commerce',
  'IOT',
  'Financial services',
  'Media & Entertainment',
  'Communications',
  'Other',
]

const SizeList = ['Just me', '2-50', '51-200', '201-500', '500+']

const RoleList = ['Developer', 'Designer', 'Product Manager', 'Other']

const ReferenceList = ['Word of mouth', 'Google', 'Social Media', 'Other']
