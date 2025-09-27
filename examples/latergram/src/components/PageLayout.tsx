import React, { useState } from 'react';
import { Outlet } from 'react-router-dom';
import { MessageCircle, X } from 'lucide-react';
import { Drawer, DrawerContent, DrawerTrigger, DrawerClose } from './ui/drawer';
import AgentChat from '../views/AgentChat';

export const PageLayout: React.FC = () => {
  const [open, setOpen] = useState(false);

  return (
    <Drawer open={open} onOpenChange={setOpen} direction="bottom">
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
      <DrawerContent className="max-h-[85vh] h-[85vh] p-0 rounded-t-lg overflow-clip">
        <div className="flex flex-col h-full -mt-6">
          {/* Drawer Header with Close Button */}
          <div className="flex items-center justify-between border-b border-gray-200 bg-white">
            <div className="flex flex-grow">
            </div>
            <DrawerClose asChild>
              <button
                type="button"
                className="p-2 hover:bg-gray-100 rounded-full transition-colors"
                aria-label="Close drawer"
              >
                <X className="w-5 h-5 text-gray-600" />
              </button>
            </DrawerClose>
          </div>

          {/* AgentChat Component */}
          <div className="flex-1 overflow-hidden">
            <AgentChat />
          </div>
        </div>
      </DrawerContent>
    </Drawer>
  );
};
