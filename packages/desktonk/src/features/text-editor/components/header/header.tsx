import { ArrowLeft, DownloadIcon, Share2Icon } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { PresenceIndicators } from '@/features/presence';
import { Button } from '../../../../components/ui/button/button';
import { EditableTitle } from './editable-title';

export default function Header() {
  const navigate = useNavigate();

  return (
    <nav className="flex flex-row items-center justify-between w-full">
      {/* Left - Back button */}
      <div className="flex-1 flex">
        <Button
          variant="ghost"
          size="icon"
          onClick={() => navigate('/')}
          aria-label="Back to Desktop"
          className="shrink-0"
        >
          <ArrowLeft />
        </Button>
      </div>

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
