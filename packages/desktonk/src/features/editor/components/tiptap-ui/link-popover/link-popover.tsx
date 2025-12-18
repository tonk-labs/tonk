'use client';

import type { Editor } from '@tiptap/react';
// --- Icons ---
import { CornerDownLeft, ExternalLink, Link, Trash2 } from 'lucide-react';
import * as React from 'react';
// --- Tiptap UI ---
import type { UseLinkPopoverConfig } from '@/features/editor/components/tiptap-ui/link-popover';
import { useLinkPopover } from '@/features/editor/components/tiptap-ui/link-popover';
// --- UI Primitives ---
import type { ButtonProps } from '@/features/editor/components/tiptap-ui-primitive/button';
import {
  Button,
  ButtonGroup,
} from '@/features/editor/components/tiptap-ui-primitive/button';
import {
  Card,
  CardBody,
  CardItemGroup,
} from '@/features/editor/components/tiptap-ui-primitive/card';
import {
  Input,
  InputGroup,
} from '@/features/editor/components/tiptap-ui-primitive/input';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/features/editor/components/tiptap-ui-primitive/popover';
import { Separator } from '@/features/editor/components/tiptap-ui-primitive/separator';
// --- Hooks ---
import { useIsMobile } from '@/hooks/use-mobile';
import { useTiptapEditor } from '@/hooks/use-tiptap-editor';

export interface LinkMainProps {
  /**
   * The URL to set for the link.
   */
  url: string;
  /**
   * Function to update the URL state.
   */
  setUrl: React.Dispatch<React.SetStateAction<string | null>>;
  /**
   * Function to set the link in the editor.
   */
  setLink: () => void;
  /**
   * Function to remove the link from the editor.
   */
  removeLink: () => void;
  /**
   * Function to open the link.
   */
  openLink: () => void;
  /**
   * Whether the link is currently active in the editor.
   */
  isActive: boolean;
  /**
   * Callback to close the popover
   */
  onClose?: () => void;
}

export interface LinkPopoverProps
  extends Omit<ButtonProps, 'type'>,
    UseLinkPopoverConfig {
  /**
   * Callback for when the popover opens or closes.
   */
  onOpenChange?: (isOpen: boolean) => void;
  /**
   * Whether to automatically open the popover when a link is active.
   * @default true
   */
  autoOpenOnLinkActive?: boolean;
}

/**
 * Link button component for triggering the link popover
 */
export const LinkButton = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, children, ...props }, ref) => {
    return (
      <Button
        type="button"
        className={className}
        data-style="ghost"
        role="button"
        tabIndex={-1}
        aria-label="Link"
        tooltip="Link"
        ref={ref}
        {...props}
      >
        {children || <Link className="tiptap-button-icon" />}
      </Button>
    );
  }
);

LinkButton.displayName = 'LinkButton';

/**
 * Main content component for the link popover
 */
const LinkMain: React.FC<LinkMainProps> = ({
  url,
  setUrl,
  setLink,
  removeLink,
  openLink,
  isActive,
  onClose,
}) => {
  const isMobile = useIsMobile();

  const handleKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      setLink();
    }
  };

  const handleRemoveLinkAndClose = React.useCallback(() => {
    removeLink();
    onClose?.();
  }, [removeLink, onClose]);

  const handleOpenLinkAndClose = React.useCallback(() => {
    openLink();
    onClose?.();
  }, [openLink, onClose]);

  return (
    <Card
      style={{
        ...(isMobile ? { boxShadow: 'none', border: 0 } : {}),
      }}
    >
      <CardBody
        style={{
          ...(isMobile ? { padding: 0 } : {}),
        }}
      >
        <CardItemGroup orientation="horizontal">
          <InputGroup>
            <Input
              type="url"
              placeholder="Paste a link..."
              value={url}
              onChange={e => setUrl(e.target.value)}
              onKeyDown={handleKeyDown}
              autoFocus
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="off"
            />
          </InputGroup>

          <ButtonGroup orientation="horizontal">
            <Button
              type="button"
              onClick={setLink}
              title="Apply link"
              disabled={!url && !isActive}
              data-style="ghost"
            >
              <CornerDownLeft className="tiptap-button-icon" />
            </Button>
          </ButtonGroup>

          <Separator />

          <ButtonGroup orientation="horizontal">
            <Button
              type="button"
              onClick={handleOpenLinkAndClose}
              title="Open in new window"
              disabled={!url && !isActive}
              data-style="ghost"
            >
              <ExternalLink className="tiptap-button-icon" />
            </Button>

            <Button
              type="button"
              onClick={handleRemoveLinkAndClose}
              title="Remove link"
              disabled={!url && !isActive}
              data-style="ghost"
            >
              <Trash2 className="tiptap-button-icon" />
            </Button>
          </ButtonGroup>
        </CardItemGroup>
      </CardBody>
    </Card>
  );
};

/**
 * Link content component for standalone use
 */
export const LinkContent: React.FC<{
  editor?: Editor | null;
}> = ({ editor }) => {
  const linkPopover = useLinkPopover({
    editor,
  });

  return <LinkMain {...linkPopover} />;
};

/**
 * Link popover component for Tiptap editors.
 *
 * For custom popover implementations, use the `useLinkPopover` hook instead.
 */
export const LinkPopover = React.forwardRef<
  HTMLButtonElement,
  LinkPopoverProps
>(
  (
    {
      editor: providedEditor,
      hideWhenUnavailable = false,
      onSetLink,
      onOpenChange,
      autoOpenOnLinkActive = true,
      onClick,
      children,
      ...buttonProps
    },
    ref
  ) => {
    const { editor } = useTiptapEditor(providedEditor);
    const [isOpen, setIsOpen] = React.useState(false);

    const {
      isVisible,
      canSet,
      isActive,
      url,
      setUrl,
      setLink,
      removeLink,
      openLink,
      label,
      Icon,
    } = useLinkPopover({
      editor,
      hideWhenUnavailable,
      onSetLink,
    });

    const handleOnOpenChange = React.useCallback(
      (nextIsOpen: boolean) => {
        setIsOpen(nextIsOpen);
        onOpenChange?.(nextIsOpen);
      },
      [onOpenChange]
    );

    const handleSetLink = React.useCallback(() => {
      setLink();
      setIsOpen(false);
    }, [setLink]);

    const handleClick = React.useCallback(
      (event: React.MouseEvent<HTMLButtonElement>) => {
        onClick?.(event);
        if (event.defaultPrevented) return;
        setIsOpen(!isOpen);
      },
      [onClick, isOpen]
    );

    React.useEffect(() => {
      if (autoOpenOnLinkActive && isActive) {
        setIsOpen(true);
      }
    }, [autoOpenOnLinkActive, isActive]);

    if (!isVisible) {
      return null;
    }

    return (
      <Popover open={isOpen} onOpenChange={handleOnOpenChange}>
        <PopoverTrigger asChild>
          <LinkButton
            disabled={!canSet}
            data-active-state={isActive ? 'on' : 'off'}
            data-disabled={!canSet}
            aria-label={label}
            aria-pressed={isActive}
            onClick={handleClick}
            {...buttonProps}
            ref={ref}
          >
            {children ?? <Icon className="tiptap-button-icon" />}
          </LinkButton>
        </PopoverTrigger>

        <PopoverContent onInteractOutside={() => setIsOpen(false)}>
          <LinkMain
            url={url}
            setUrl={setUrl}
            setLink={handleSetLink}
            removeLink={removeLink}
            openLink={openLink}
            isActive={isActive}
            onClose={() => setIsOpen(false)}
          />
        </PopoverContent>
      </Popover>
    );
  }
);

LinkPopover.displayName = 'LinkPopover';

export default LinkPopover;
