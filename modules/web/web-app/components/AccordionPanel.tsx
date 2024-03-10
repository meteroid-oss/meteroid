import { Accordion, AccordionItem, AccordionTrigger, cn } from '@ui2/components'
import { AccordionContent } from '@radix-ui/react-accordion'

interface AccordionPanelProps {
  title: string | JSX.Element
  children: React.ReactNode
  defaultOpen?: boolean
  triggerClassName?: string
}
export const AccordionPanel = ({
  title,
  children,
  defaultOpen = true,
  triggerClassName,
}: AccordionPanelProps) => {
  return (
    <Accordion type="single" collapsible defaultValue={defaultOpen ? 'item-1' : undefined}>
      <AccordionItem value="item-1" className="border-0">
        <AccordionTrigger className={cn('hover:no-underline', triggerClassName)}>
          {title}
        </AccordionTrigger>
        <AccordionContent className="pb-4">{children}</AccordionContent>
      </AccordionItem>
    </Accordion>
  )
}
