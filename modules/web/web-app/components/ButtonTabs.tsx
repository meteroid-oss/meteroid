import { Button, ButtonProps, cn } from "@ui/index"
import { PropsWithChildren } from "react"

interface ButtonTabsProps extends Omit<ButtonProps, 'variant'>, PropsWithChildren {
    active?: boolean
}

export const ButtonTabs = ({ children, active = false, ...props }: ButtonTabsProps) => {
    const { className, ...rest } = props

    return (
        <Button
            variant="ghost"
            className={cn(
                'text-[#606060] px-2 h-[26px] text-xs',
                active && 'bg-accent text-accent-foreground',
                !active && 'hover:bg-accent hover:text-accent-foreground',
                className
            )}
            {...rest}
        >
            {children}
        </Button>
    )
}