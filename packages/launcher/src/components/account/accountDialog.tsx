import { Dialog } from '@base-ui-components/react/dialog';
import type { ReactNode } from 'react';
import { FaUser } from 'react-icons/fa6';
import { cn } from '@/lib/utils';
import SideBarButton from '../sidebarButton/sidebarButton';

interface AccountDialogProps {
  title?: string;
  children?: ReactNode;
}

function AccountRoot({ title, children }: AccountDialogProps) {
  return (
    <Dialog.Root>
      <Dialog.Trigger
        render={
          <SideBarButton
            alt="Account"
            className={cn(
              'rounded-full [corner-shape:squircle]! w-10 h-10 ',
              'bg-blue-200 dark:bg-blue-900 dark:text-blue-100',
              ''
            )}
          >
            <FaUser className="text-xl" />
          </SideBarButton>
        }
      />
      <Dialog.Portal>
        <Dialog.Backdrop className="fixed inset-0 min-h-dvh bg-black opacity-20 transition-all duration-150 data-ending-style:opacity-0 data-starting-style:opacity-0 dark:opacity-70 supports-[-webkit-touch-callout:none]:absolute" />
        <Dialog.Popup className="fixed top-1/2 left-1/2 -mt-8 w-96 max-w-[calc(100vw-3rem)] -translate-x-1/2 -translate-y-1/2 rounded-lg bg-night-50 dark:bg-night-900 p-6 text-night-900 outline outline-night-200 transition-all duration-150 data-ending-style:scale-90 data-ending-style:opacity-0 data-starting-style:scale-90 data-starting-style:opacity-0 dark:outline-night-900 dark:text-night-100 font-gestalt text-base">
          {title && <Dialog.Title className="-mt-1.5 mb-1 text-lg font-bold">{title}</Dialog.Title>}
          {children}
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function DialogFooter({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 px-6 py-3 border-t border-night-200 dark:border-night-900 text-night-900 dark:text-night-100">
      {children}
    </div>
  );
}

function CloseDialog() {
  return (
    <Dialog.Close className="flex h-10 items-center justify-center rounded-md border border-night-200 dark:border-night-900 bg-night-50 dark:bg-night-800 dark:text-night-100 px-3.5 text-base font-medium text-night-900 select-none hover:bg-night-100 focus-visible:outline focus-visible:-outline-offset-1 focus-visible:outline-blue-800 active:bg-night-100">
      Close
    </Dialog.Close>
  );
}

const AccountDialog = {
  Root: AccountRoot,
  Close: CloseDialog,
  Footer: DialogFooter,
};

export default AccountDialog;
