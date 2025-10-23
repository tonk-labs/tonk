export default function Layout({children}: {children: React.ReactNode}) {
  return (
    <div className="flex flex-col h-full">
      <div className="flex-1 flex flex-col">
        {children}
      </div>
    </div>
  );
}