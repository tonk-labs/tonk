import { DownloadIcon, Share2Icon } from 'lucide-react';
import { Button } from '../ui/button/button';
import { PresenceIndicators } from '@/features/presence';
import { EditableTitle } from './editable-title';

export default function Header() {
  return (
    <nav className="flex flex-row items-center justify-between w-full">
      {/* Left spacer - ensures center alignment */}
      <div className="flex-1 flex" />

      {/* Center - Editable title */}
      <div className="flex-1 flex justify-center">
        <EditableTitle />
      </div>

      {/* Right - buttons and presence */}
      <div className="flex-1 flex justify-end items-center gap-2">
        <div className="flex flex-col justify-end items-end gap-2">
        <div className="flex flex-row justify-end items-end gap-2">
        <Button>
          <DownloadIcon />
        </Button>
        <Button>
          Share
          <Share2Icon />
        </Button>
</div>
        <PresenceIndicators maxVisible={5} />
        </div>
      </div>
    </nav>
  );
}
