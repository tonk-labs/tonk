import { Code2Icon, MessageCircle } from 'lucide-react';
import type React from 'react';
import { createRef, useCallback, useEffect, useState } from 'react';
import { Link, Outlet } from 'react-router-dom';
import { useAgentChat } from '../lib/agent/use-agent-chat';
import { cn, isMobile } from '../lib/utils';
import AgentChat from './chat/AgentChat';
import { Drawer, DrawerContent, DrawerTrigger } from './ui/drawer';
import { useVisualViewport } from './hooks/useVisualViewport';

interface PageLayoutProps {
  viewPath: string;
}

export const PageLayout: React.FC<PageLayoutProps> = ({ viewPath }) => {
  const [open, setOpen] = useState(false);
  const [isDesktop, setIsDesktop] = useState(false);
  const inputRef = createRef<HTMLTextAreaElement>();
  const { isLoading } = useAgentChat();
  const {
    height: viewportHeight,
    isKeyboardOpen,
    keyboardHeight,
  } = useVisualViewport();
  const isMobileDevice = isMobile();

  useEffect(() => {
    const checkScreenSize = () => {
      setIsDesktop(window.innerWidth >= 1024);
    };

    checkScreenSize();
    window.addEventListener('resize', checkScreenSize);
    return () => window.removeEventListener('resize', checkScreenSize);
  }, []);

  const handleOpen = useCallback((shouldOpen: boolean) => {
    setOpen(shouldOpen);
  }, []);

  const drawerMaxHeight =
    isMobileDevice && isKeyboardOpen
      ? `${viewportHeight}px`
      : isDesktop
        ? '100vh'
        : '85vh';

  const buttonBottomPosition =
    isMobileDevice && isKeyboardOpen && open
      ? `${keyboardHeight + 16}px`
      : '1.5rem';

  return (
    <Drawer
      open={open}
      onOpenChange={handleOpen}
      direction={isDesktop ? 'right' : 'bottom'}
      shouldScaleBackground={!isMobileDevice}
      modal={!isMobileDevice}
      dismissible={isDesktop}
    >
      <div className="min-h-screen">
        <Outlet />
      </div>

      <DrawerTrigger asChild>
        <div
          className={cn(
            'fixed z-[1000] flex flex-row gap-2 transition-all duration-300',
            isDesktop && open ? 'right-[450px]' : 'right-6'
          )}
          style={{
            bottom: buttonBottomPosition,
          }}
        >
          <button
            type="button"
            className={cn(
              'w-16 h-16 relative hover:scale-110 transition-all flex items-center justify-center',
              isLoading && 'animate-bounce'
            )}
            aria-label="Open AI Assistant"
          >
            <div
              className={`absolute inset-0 ${isLoading ? 'animate-spin' : ''}`}
              style={{
                background:
                  'linear-gradient(90deg, #E6C767 0%, #8DBEC8 25%, #A8D8B9 50%, #F4A7A7 75%, #E6C767 100%)',
                backgroundSize: '200% 100%',
                animation: isLoading
                  ? 'spin 5s linear infinite'
                  : 'rainbowSlide 4s linear infinite',
                WebkitMaskImage:
                  "url(\"data:image/svg+xml,%3Csvg viewBox='0 0 48 48' xmlns='http://www.w3.org/2000/svg'%3E%3Cpath d='M42.3,24l3.4-5.1a2,2,0,0,0,.2-1.7A1.8,1.8,0,0,0,44.7,16l-5.9-2.4-.5-5.9a2.1,2.1,0,0,0-.7-1.5,2,2,0,0,0-1.7-.3L29.6,7.2,25.5,2.6a2.2,2.2,0,0,0-3,0L18.4,7.2,12.1,5.9a2,2,0,0,0-1.7.3,2.1,2.1,0,0,0-.7,1.5l-.5,5.9L3.3,16a1.8,1.8,0,0,0-1.2,1.2,2,2,0,0,0,.2,1.7L5.7,24,2.3,29.1a2,2,0,0,0,1,2.9l5.9,2.4.5,5.9a2.1,2.1,0,0,0,.7,1.5,2,2,0,0,0,1.7.3l6.3-1.3,4.1,4.5a2,2,0,0,0,3,0l4.1-4.5,6.3,1.3a2,2,0,0,0,1.7-.3,2.1,2.1,0,0,0,.7-1.5l.5-5.9L44.7,32a2,2,0,0,0,1-2.9Z'/%3E%3C/svg%3E\")",
                maskImage:
                  "url(\"data:image/svg+xml,%3Csvg viewBox='0 0 48 48' xmlns='http://www.w3.org/2000/svg'%3E%3Cpath d='M42.3,24l3.4-5.1a2,2,0,0,0,.2-1.7A1.8,1.8,0,0,0,44.7,16l-5.9-2.4-.5-5.9a2.1,2.1,0,0,0-.7-1.5,2,2,0,0,0-1.7-.3L29.6,7.2,25.5,2.6a2.2,2.2,0,0,0-3,0L18.4,7.2,12.1,5.9a2,2,0,0,0-1.7.3,2.1,2.1,0,0,0-.7,1.5l-.5,5.9L3.3,16a1.8,1.8,0,0,0-1.2,1.2,2,2,0,0,0,.2,1.7L5.7,24,2.3,29.1a2,2,0,0,0,1,2.9l5.9,2.4.5,5.9a2.1,2.1,0,0,0,.7,1.5,2,2,0,0,0,1.7.3l6.3-1.3,4.1,4.5a2,2,0,0,0,3,0l4.1-4.5,6.3,1.3a2,2,0,0,0,1.7-.3,2.1,2.1,0,0,0,.7-1.5l.5-5.9L44.7,32a2,2,0,0,0,1-2.9Z'/%3E%3C/svg%3E\")",
                WebkitMaskSize: '100%',
                maskSize: '100%',
                WebkitMaskPosition: 'center',
                maskPosition: 'center',
                WebkitMaskRepeat: 'no-repeat',
                maskRepeat: 'no-repeat',
              }}
            />
            <MessageCircle className="w-7 h-7 text-white relative z-10 drop-shadow-md" />
          </button>
          <Link
            title="Open Page Editor"
            to={`/editor/pages?file=${encodeURIComponent(viewPath)}`}
            className="w-16 h-16 items-center gap-2 px-4 py-2 bg-white border border-gray-200 rounded-full hover:bg-gray-50 transition-colors shadow-sm hidden lg:inline-flex"
          >
            <Code2Icon className="w-full h-full" />
          </Link>
        </div>
      </DrawerTrigger>

      <DrawerContent
        title="Agent Chat"
        className={cn(
          'p-0 overflow-clip z-[1000000]',
          isDesktop ? 'w-[600px] h-full rounded-l-lg' : 'rounded-t-lg'
        )}
        style={{
          maxHeight: drawerMaxHeight,
          height: drawerMaxHeight,
        }}
        onWheel={e => e.stopPropagation()}
      >
        <div
          className="flex flex-col flex-1 bg-white min-h-0"
          style={{
            touchAction: 'pan-y',
          }}
        >
          <AgentChat inputRef={inputRef} onClose={() => handleOpen(false)} />
        </div>
      </DrawerContent>
    </Drawer>
  );
};
