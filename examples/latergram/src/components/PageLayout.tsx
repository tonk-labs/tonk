import React, { createRef, useCallback, useRef, useState } from 'react';
import { Outlet } from 'react-router-dom';
import { MessageCircle, X } from 'lucide-react';
import { Drawer, DrawerContent, DrawerTrigger, DrawerClose } from './ui/drawer';
import AgentChat from '../views/AgentChat';
import { useAgentStore } from '../lib/agent/use-agent-store';
import { cn } from '../lib/utils';

interface PageLayoutProps {
  viewPath: string;
}

export const PageLayout: React.FC<PageLayoutProps> = ({ viewPath }) => {
  const [open, setOpen] = useState(false);
  const inputRef = createRef<HTMLTextAreaElement>();
  const { isLoading } = useAgentStore();

  const handleOpen = useCallback((shouldOpen: boolean) => {
    setOpen(shouldOpen);
  }, []);

  return (
    <Drawer open={open} onOpenChange={handleOpen} direction="bottom">
      {/* Page content */}
      <div className="min-h-screen">
        <Outlet />
      </div>

      {/* Floating Action Button */}
      <DrawerTrigger asChild>
        <div className="fixed bottom-6 right-6 w-16 h-16 z-[1000]">
          <button
            type="button"
            className={cn("relative w-full h-full hover:scale-110 transition-all flex items-center justify-center", isLoading && 'animate-bounce')}
            aria-label="Open AI Assistant"
          >
            {/* Star burst background with rainbow gradient */}
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
            {/* Message icon on top */}
            <MessageCircle className="w-7 h-7 text-white relative z-10 drop-shadow-md" />
          </button>
        </div>
      </DrawerTrigger>

      {/* Drawer Content */}
      <DrawerContent
        title='Agent Chat'
        className="max-h-[85vh] h-[85vh] p-0 rounded-t-lg overflow-clip z-[1000000]"
        onWheel={e => e.stopPropagation()}
      >
        <div className="flex flex-col h-full -mt-6 bg-white">
          {/* AgentChat Component */}
          {/* <div className="flex-1 overflow-hidden"> */}
            <AgentChat inputRef={inputRef} />
          {/* </div> */}
        </div>
      </DrawerContent>
    </Drawer>
  );
};
