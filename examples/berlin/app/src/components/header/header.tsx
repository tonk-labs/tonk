import { DownloadIcon, Share2Icon } from 'lucide-react';
import { Button } from '../ui/button/button';
import { PresenceIndicators } from '@/features/presence';
import { EditableTitle } from './editable-title';

export default function Header() {
  return (
    <nav className="flex items-center justify-between w-full">
      {/* Left spacer - ensures center alignment */}
      <div className="flex-1" />

      {/* Center - Editable title */}
      <div className="flex-1 flex justify-center">
        <EditableTitle />
      </div>

      {/* Right - buttons and presence */}
      <div className="flex-1 flex justify-end items-center gap-2">
        <PresenceIndicators maxVisible={5} />
        <Button>
          <DownloadIcon />
        </Button>
        <Button>
          Share
          <Share2Icon />
        </Button>
      </div>
    </nav>
  );
}
