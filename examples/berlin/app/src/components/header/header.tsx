import { DownloadIcon, Share2Icon } from 'lucide-react';
import { Button } from '../ui/button/button';
import { PresenceIndicators } from '@/features/presence';

export default function Header() {
  return (
    <nav className="">
      
        <div className="flex flex-row items-end gap-2">
          <Button>
            <DownloadIcon />
          </Button>
          <Button>
            Share
            <Share2Icon />
          </Button>
        </div>
        <div className="flex flex-row items-end gap-2">
        <PresenceIndicators maxVisible={5} />
        </div>
    </nav>
  );
}
