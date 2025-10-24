import * as React from "react"

// --- Icons ---
import { ChevronDownIcon } from "@/features/editor/components/tiptap-icons/chevron-down-icon"

// --- Hooks ---
import { useTiptapEditor } from "@/hooks/use-tiptap-editor"

// --- Tiptap UI ---
import { HeadingButton } from "@/features/editor/components/tiptap-ui/heading-button"
import type { UseHeadingDropdownMenuConfig } from "@/features/editor/components/tiptap-ui/heading-dropdown-menu"
import { useHeadingDropdownMenu } from "@/features/editor/components/tiptap-ui/heading-dropdown-menu"

// --- UI Primitives ---
import type { ButtonProps } from "@/features/editor/components/tiptap-ui-primitive/button"
import { Button, ButtonGroup } from "@/features/editor/components/tiptap-ui-primitive/button"
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@/features/editor/components/tiptap-ui-primitive/dropdown-menu"
import { Card, CardBody } from "@/features/editor/components/tiptap-ui-primitive/card"
import { useDropdownMenuState } from "@/features/editor/components/tiptap-ui-primitive/dropdown-menu/dropdown-menu-context"

export interface HeadingDropdownMenuProps
  extends Omit<ButtonProps, "type">,
    UseHeadingDropdownMenuConfig {
  /**
   * Whether to render the dropdown menu in a portal
   * @default false
   */
  portal?: boolean
  /**
   * Callback for when the dropdown opens or closes
   */
  onOpenChange?: (isOpen: boolean) => void
}

/**
 * Dropdown menu component for selecting heading levels in a Tiptap editor.
 *
 * For custom dropdown implementations, use the `useHeadingDropdownMenu` hook instead.
 */
export const HeadingDropdownMenu = React.forwardRef<
  HTMLButtonElement,
  HeadingDropdownMenuProps
>(
  (
    {
      editor: providedEditor,
      levels = [1, 2, 3, 4, 5, 6],
      hideWhenUnavailable = false,
      portal = false,
      onOpenChange,
      ...buttonProps
    },
    ref
  ) => {
    const { editor } = useTiptapEditor(providedEditor)
    const { isOpen, setIsOpen } = useDropdownMenuState("heading-dropdown")
    const { isVisible, isActive, canToggle, Icon } = useHeadingDropdownMenu({
      editor,
      levels,
      hideWhenUnavailable,
    })

    const handleOpenChange = React.useCallback(
      (open: boolean) => {
        if (!editor || !canToggle) return
        setIsOpen(open)
        onOpenChange?.(open)
      },
      [canToggle, editor, onOpenChange]
    )

    const handleClose = React.useCallback(() => {
      setIsOpen(false)
    }, [])

    if (!isVisible) {
      return null
    }

    return (
      <DropdownMenu open={isOpen} onOpenChange={handleOpenChange}>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            data-style="ghost"
            data-active-state={isActive ? "on" : "off"}
            role="button"
            tabIndex={-1}
            disabled={!canToggle}
            data-disabled={!canToggle}
            aria-label="Format text as heading"
            aria-pressed={isActive}
            tooltip="Heading"
            {...buttonProps}
            ref={ref}
          >
            <Icon className="tiptap-button-icon" />
            <ChevronDownIcon className="tiptap-button-dropdown-small" />
          </Button>
        </DropdownMenuTrigger>

        <DropdownMenuContent
          align="start"
          portal={portal}
          onInteractOutside={() => setIsOpen(false)}
        >
          <Card>
            <CardBody>
              <ButtonGroup>
                {levels.map((level) => (
                  <DropdownMenuItem
                    key={`heading-${level}`}
                    onSelect={() => {
                      handleClose()
                    }}
                    asChild
                  >
                    <HeadingButton
                      editor={editor}
                      level={level}
                      text={`Heading ${level}`}
                      showTooltip={false}
                    />
                  </DropdownMenuItem>
                ))}
              </ButtonGroup>
            </CardBody>
          </Card>
        </DropdownMenuContent>
      </DropdownMenu>
    )
  }
)

HeadingDropdownMenu.displayName = "HeadingDropdownMenu"

export default HeadingDropdownMenu
