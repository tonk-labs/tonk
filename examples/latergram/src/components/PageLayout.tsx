import React, { createRef, useCallback, useRef, useState } from 'react';
import { Outlet } from 'react-router-dom';
import { MessageCircle, X } from 'lucide-react';
import { Drawer, DrawerContent, DrawerTrigger, DrawerClose } from './ui/drawer';
import AgentChat from '../views/AgentChat';

interface PageLayoutProps {
  viewPath: string;
}

export const PageLayout: React.FC<PageLayoutProps> = ({viewPath}) => {
  const [open, setOpen] = useState(false);
  const inputRef = createRef<HTMLTextAreaElement>();

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
        <button
          type="button"
          className="fixed bottom-6 right-6 z-40 w-14 h-14 bg-blue-500 text-white rounded-full shadow-lg hover:bg-blue-600 transition-all hover:scale-110 flex items-center justify-center"
          aria-label="Open AI Assistant"
        >
          <MessageCircle className="w-6 h-6" />
        </button>
      </DrawerTrigger>

      {/* Drawer Content */}
      <DrawerContent className="max-h-[85vh] h-[85vh] p-0 rounded-t-lg overflow-clip" onWheel={(e)=>e.stopPropagation()}>
        <div className="flex flex-col h-full -mt-6 bg-white">
          {/* Drawer Header with Close Button */}

          {/* AgentChat Component */}
          <div className="flex-1 overflow-hidden">
            <AgentChat inputRef={inputRef}/>
          </div>
          <div className="absolute top-0 left-0 flex items-center justify-between w-full h-auto">
            <DrawerClose asChild>
              <button
                type="button"
                className="p-2 hover:bg-gray-100 rounded-full transition-colors absolute top-0 right-0"
                aria-label="Close drawer"
              >
                <X className="w-5 h-5 text-gray-600" />
              </button>
            </DrawerClose>
          </div>
        </div>
      </DrawerContent>
    </Drawer>
  );
};
