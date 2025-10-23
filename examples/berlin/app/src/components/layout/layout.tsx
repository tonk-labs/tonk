import './layout.css';

export default function Layout({children}: {children: React.ReactNode}) {
  return (
    <main id="main-page" className="flex flex-col h-full">
        <nav id="nav-topbar"> </nav>
        {children}
    </main>
  );
}