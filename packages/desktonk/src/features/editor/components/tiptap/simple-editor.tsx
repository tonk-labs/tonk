'use client';

import * as React from 'react';
import { EditorContent, EditorContext, type Editor } from '@tiptap/react';

// --- UI Primitives ---
import { Button } from '@/features/editor/components/tiptap-ui-primitive/button';
import { Spacer } from '@/features/editor/components/tiptap-ui-primitive/spacer';
import {
  Toolbar,
  ToolbarGroup,
  ToolbarSeparator,
} from '@/features/editor/components/tiptap-ui-primitive/toolbar';

// --- Tiptap UI ---
import { HeadingDropdownMenu } from '@/features/editor/components/tiptap-ui/heading-dropdown-menu';
import { ImageUploadButton } from '@/features/editor/components/tiptap-ui/image-upload-button';
import { ListDropdownMenu } from '@/features/editor/components/tiptap-ui/list-dropdown-menu';
import { BlockquoteButton } from '@/features/editor/components/tiptap-ui/blockquote-button';
import { CodeBlockButton } from '@/features/editor/components/tiptap-ui/code-block-button';
import {
  ColorHighlightPopover,
  ColorHighlightPopoverContent,
  ColorHighlightPopoverButton,
} from '@/features/editor/components/tiptap-ui/color-highlight-popover';
import {
  LinkPopover,
  LinkContent,
  LinkButton,
} from '@/features/editor/components/tiptap-ui/link-popover';
import { MarkButton } from '@/features/editor/components/tiptap-ui/mark-button';
import { TextAlignButton } from '@/features/editor/components/tiptap-ui/text-align-button';
import { UndoRedoButton } from '@/features/editor/components/tiptap-ui/undo-redo-button';

// --- Icons ---
import { ArrowLeft, Highlighter, Link } from 'lucide-react';

// --- Hooks ---
import { useIsMobile } from '@/hooks/use-mobile';
import { useWindowSize } from '@/hooks/use-window-size';
import { useCursorVisibility } from '@/hooks/use-cursor-visibility';
import { useEffect } from 'react';

const MainToolbarContent = ({
  onHighlighterClick,
  onLinkClick,
  isMobile,
}: {
  onHighlighterClick: () => void;
  onLinkClick: () => void;
  isMobile: boolean;
}) => {
  return (
    <>
      {/* <Spacer /> */}

      <ToolbarGroup>
        <UndoRedoButton action="undo" />
        <UndoRedoButton action="redo" />
      </ToolbarGroup>

      <ToolbarSeparator />

      <ToolbarGroup>
        <HeadingDropdownMenu levels={[1, 2, 3, 4]} portal={isMobile} />
        <ListDropdownMenu types={['bulletList', 'orderedList', 'taskList']} portal={isMobile} />
        <BlockquoteButton />
        <CodeBlockButton />
      </ToolbarGroup>

      <ToolbarSeparator />

      <ToolbarGroup>
        <MarkButton type="bold" />
        <MarkButton type="italic" />
        <MarkButton type="strike" />
        <MarkButton type="code" />
        <MarkButton type="underline" />
        {!isMobile ? (
          <ColorHighlightPopover />
        ) : (
          <ColorHighlightPopoverButton onClick={onHighlighterClick} />
        )}
        {!isMobile ? <LinkPopover /> : <LinkButton onClick={onLinkClick} />}
      </ToolbarGroup>

      <ToolbarSeparator />

      <ToolbarGroup>
        <MarkButton type="superscript" />
        <MarkButton type="subscript" />
      </ToolbarGroup>

      <ToolbarSeparator />

      <ToolbarGroup>
        <TextAlignButton align="left" />
        <TextAlignButton align="center" />
        <TextAlignButton align="right" />
        <TextAlignButton align="justify" />
      </ToolbarGroup>

      <ToolbarSeparator />

      <ToolbarGroup>
        <ImageUploadButton text="Add" />
      </ToolbarGroup>

      <Spacer />

      {isMobile && <ToolbarSeparator />}
      {/* 
      <ToolbarGroup>
        <ThemeToggle />
      </ToolbarGroup> */}
    </>
  );
};

const MobileToolbarContent = ({
  type,
  onBack,
}: {
  type: 'highlighter' | 'link';
  onBack: () => void;
}) => (
  <>
    <ToolbarGroup>
      <Button data-style="ghost" onClick={onBack}>
        <ArrowLeft className="tiptap-button-icon" />
        {type === 'highlighter' ? (
          <Highlighter className="tiptap-button-icon" />
        ) : (
          <Link className="tiptap-button-icon" />
        )}
      </Button>
    </ToolbarGroup>

    <ToolbarSeparator />

    {type === 'highlighter' ? <ColorHighlightPopoverContent /> : <LinkContent />}
  </>
);

interface SimpleEditorProps {
  editor: Editor | null;
}

export function SimpleEditor({ editor }: SimpleEditorProps) {
  const isMobile = useIsMobile();
  const { height } = useWindowSize();
  const [mobileView, setMobileView] = React.useState<'main' | 'highlighter' | 'link'>('main');
  const toolbarRef = React.useRef<HTMLDivElement>(null);

  const rect = useCursorVisibility({
    editor,
    overlayHeight: toolbarRef.current?.getBoundingClientRect().height ?? 0,
  });

  useEffect(() => {
    if (!isMobile && mobileView !== 'main') {
      setMobileView('main');
    }
  }, [isMobile, mobileView]);

  if (!editor) {
    return null;
  }

  return (
    <EditorContext.Provider value={{ editor }}>
      <div className="editor-container">
        <div className="toolbar-wrapper">
          <Toolbar
            ref={toolbarRef}
            style={{
              ...(isMobile
                ? {
                    bottom: `calc(100% - ${height - rect.y}px)`,
                  }
                : {}),
            }}
          >
            {mobileView === 'main' ? (
              <MainToolbarContent
                onHighlighterClick={() => setMobileView('highlighter')}
                onLinkClick={() => setMobileView('link')}
                isMobile={isMobile}
              />
            ) : (
              <MobileToolbarContent
                type={mobileView === 'highlighter' ? 'highlighter' : 'link'}
                onBack={() => setMobileView('main')}
              />
            )}
          </Toolbar>
        </div>
      </div>

      <div className="editor-container">
        <article id="editor-area">
          <EditorContent editor={editor} role="presentation" className="simple-editor-content" />
        </article>
      </div>
    </EditorContext.Provider>
  );
}
