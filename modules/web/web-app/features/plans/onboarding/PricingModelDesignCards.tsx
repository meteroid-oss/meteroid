import { Input, Slider, Switch, cn } from '@md/ui'
import { ReactNode, useId, useState } from 'react'

const HeroBase = ({
  title,
  items,
  active,
}: {
  title: string
  items: ReactNode[]
  active: boolean
}) => {
  return (
    <div className="flex items-end justify-center  ">
      <div className="border-l border-t border-primary p-4 shadow-lg rounded-tl-lg w-[15%]  h-[70px]"></div>
      <div className="border-t border-x border-primary group-hover:border-brand p-2 shadow-lg rounded-lg rounded-b-none w-[50%] h-[100px] ">
        <p className="text-center font-bold  mb-2">{title}</p>

        {items.map((item, index) => (
          <p
            className={cn('text-center font-medium text-xs mb-2', active && 'text-brand')}
            key={index}
          >
            {item}
          </p>
        ))}
      </div>
      <div className="border-r border-t border-primary p-4 shadow-lg rounded-tr-lg max-w-[15%] grow h-[70px]"></div>
    </div>
  )
}

interface PeriodPickerProps {
  isAnnual: boolean
  setIsAnnual: (isAnnual: boolean) => void
  className?: string
}
const PeriodPicker = ({ isAnnual, setIsAnnual, className }: PeriodPickerProps) => {
  const id = useId()

  return (
    <div className={cn('flex flex-row justify-center text-xs gap-1 items-center', className)}>
      <Switch
        id={id}
        thumbClassName="h-3 w-3 group-hover:ring-1 group-hover:ring-brand"
        className="h-4 w-8  "
        checked={isAnnual}
        onCheckedChange={setIsAnnual}
        tabIndex={-1}
      />
      <label className="font-semibold" htmlFor={id}>
        Annual billing
      </label>
      <span className="">(-20%)</span>
    </div>
  )
}

const CapacitySlidersLine = () => (
  <div className="absolute inset-0 flex items-center" aria-hidden="true">
    <div className="h-[3px] w-full bg-foreground" />
  </div>
)

interface CapacitySlidersProps {
  steps: string[]
  onHover: (idx: number | undefined) => void
  selectedIdx: number
}
const CapacitySliders = ({ steps, onHover, selectedIdx }: CapacitySlidersProps) => {
  return (
    <ol role="list" className="flex items-center justify-center">
      <li key="before" className="pr-4 relative">
        <CapacitySlidersLine />
      </li>
      {steps.map((step, stepIdx) => (
        <li
          key={stepIdx}
          className={cn(
            stepIdx !== steps.length - 1 ? 'pr-8 sm:pr-12 xl:pr-14' : '',
            'relative hover:text-brand'
          )}
          onMouseEnter={() => onHover(stepIdx)}
          onMouseLeave={() => onHover(undefined)}
        >
          <>
            <CapacitySlidersLine />
            <span
              className={cn(
                'relative flex h-2.5 w-2.5 items-center justify-center rounded-full',
                stepIdx !== selectedIdx
                  ? 'border-2 border-foreground bg-background hover:bg-brand hover:border-brand'
                  : 'bg-foreground hover:bg-brand group-hover:ring-1 group-hover:ring-brand'
              )}
            >
              <span className=" text-xs font-medium absolute -rotate-45 -inset-y-9 inset-x-1 flex text-end place-self-end py-2">
                {step}
              </span>
            </span>
          </>
        </li>
      ))}
      <li key="after" className="pr-4 relative">
        <CapacitySlidersLine />
      </li>
    </ol>
  )
}

const DesignCard = ({ children }: { children: React.ReactNode }) => {
  return (
    <div className="w-full h-[180px] justify-between relative flex flex-col gap-2 py-2 m-1 bg-popover rounded-xl">
      {children}
    </div>
  )
}

export const CapacityDesignCard = () => {
  const [hovered, setHovered] = useState<number | undefined>()

  const options = [
    ['0', '1.000', '1K'],
    ['30', '10.000', '10K'],
    ['99', '50.000', '50K'],
    ['300', '200.000', '200K'],
  ]

  const selected = 2

  const optionHovered = options[hovered ?? selected]

  const heroBaseProps = {
    title: `Scale ${optionHovered[2]}`,
    items: [`$${optionHovered[0]}/month`, `${optionHovered[1]} included`],
    active: hovered != undefined && hovered != selected,
  }

  return (
    <DesignCard>
      <CapacitySliders steps={options.map(a => a[1])} onHover={setHovered} selectedIdx={selected} />
      <HeroBase {...heroBaseProps} />
    </DesignCard>
  )
}

export const RateDesignCard = () => {
  const defaultIsAnnual = false

  const [isAnnual, setIsAnnual] = useState(defaultIsAnnual)

  const options = ['$150/month', '$1.440/year']
  const optionSelected = options[Number(isAnnual)]

  const heroBaseProps = {
    title: `Scale`,
    items: [optionSelected],
    active: isAnnual != defaultIsAnnual,
  }

  return (
    <DesignCard>
      <PeriodPicker isAnnual={isAnnual} setIsAnnual={setIsAnnual} className="pt-5" />
      <HeroBase {...heroBaseProps} />
    </DesignCard>
  )
}

interface UsageSliderProps {
  baseValue: number
  maxValue: number
  unit: string
  unitPrice: number
  label: string
}

const UsageSlider = ({ baseValue, maxValue, unit, unitPrice, label }: UsageSliderProps) => {
  const [sliderValue, setSliderValue] = useState([baseValue])

  return (
    <div className="grid grid-cols-12 gap-2 text-xs justify-center items-center whitespace-nowrap w-full px-3">
      <span className="col-span-2">{label}</span>
      <span className="col-span-4">
        <Slider
          value={sliderValue}
          onValueChange={setSliderValue}
          max={maxValue}
          step={1}
          className="w-full "
          thumbClassName="group-hover:ring-1 group-hover:ring-brand "
          tabIndex={-1}
        />
      </span>

      <span className="col-span-3">
        {sliderValue[0]}
        {maxValue === sliderValue[0] && '+'} <span className="hidden xl:inline">{unit}</span>
      </span>
      <span className="col-span-3">
        {maxValue === sliderValue[0] ? (
          'Contact us'
        ) : (
          <>{Number(unitPrice * sliderValue[0]).toFixed(2)}$</>
        )}
      </span>
    </div>
  )
}

const UsageSliders = () => {
  return (
    <div className="flex flex-col gap-4 w-full ">
      <UsageSlider baseValue={1000} maxValue={9999} unit="MAU" unitPrice={0.15} label="Users" />
      <UsageSlider baseValue={200} maxValue={2000} unit="GB" unitPrice={0.2} label="Storage" />
    </div>
  )
}

export const UsageBasedDesignCard = () => {
  const heroBaseProps = {
    title: `Scale`,
    items: ['Users: 0.15$ / MAU', 'Storage: 0.2$ / GB'],
    active: false, //hovered != undefined && hovered != selected,
  }

  return (
    <DesignCard>
      <UsageSliders />
      <HeroBase {...heroBaseProps} />
    </DesignCard>
  )
}

const SlotField = ({ price }: { price: number }) => {
  const [value, setValue] = useState(10)

  return (
    <>
      <div className="flex flex-row  self-center justify-center items-center gap-2 h-6 text-sm px-6">
        <span className="font-semibold">Seats:</span>
        <Input
          type="number"
          inputMode="numeric"
          step={1}
          min={1}
          max={20}
          value={value}
          onChange={e => setValue(Number(e.target.value))}
          className="h-6 px-1 w-[60px] font-medium group-hover:ring-1 group-hover:ring-brand"
          tabIndex={-1}
        />
        <span className="col-span-3 text-xs ">
          {value >= 20 ? 'Contact us' : <>(${Number(price * value).toFixed(0)}/month)</>}
        </span>
      </div>
    </>
  )
}

export const SlotsDesignCard = () => {
  const defaultIsAnnual = false

  const [isAnnual, setIsAnnual] = useState(defaultIsAnnual)

  const options = [['$15/user/month'], ['$12/user/month', 'billed annualy']]
  const optionSelected = options[Number(isAnnual)]

  const heroBaseProps = {
    title: `Scale`,
    items: optionSelected,
    active: isAnnual != defaultIsAnnual,
  }

  return (
    <DesignCard>
      <PeriodPicker isAnnual={isAnnual} setIsAnnual={setIsAnnual} />
      <SlotField price={isAnnual ? 12 : 15} />
      <HeroBase {...heroBaseProps} />
    </DesignCard>
  )
}
