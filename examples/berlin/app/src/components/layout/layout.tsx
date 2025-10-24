import { DownloadIcon } from 'lucide-react';
import { Button } from '../ui/button/button';
import './layout.css';
import Header from '../header/header';
import { useId } from 'react';

export default function Layout({children}: {children: React.ReactNode}) {
  const headerId = useId();
  return (
    <main id={headerId} className="flex flex-col h-full m-4">
      <Header/>
        {children}
    </main>
  );
}