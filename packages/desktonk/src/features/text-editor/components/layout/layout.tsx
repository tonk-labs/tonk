import Header from '../header/header';
import { useId } from 'react';

export default function Layout({ children }: { children: React.ReactNode }) {
  const headerId = useId();
  return (
    <main id={headerId} className="rootLayout">
      <Header />
      {children}
    </main>
  );
}
