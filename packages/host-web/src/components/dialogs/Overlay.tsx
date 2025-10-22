export function Overlay({ isVisible, onClick }: { isVisible: boolean; onClick?: () => void }) {
  if (!isVisible) return null;

  return (
    <div
      onClick={onClick}
      class="fixed inset-0 bg-black/50 z-[999]"
    />
  );
}
