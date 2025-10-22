export function PromptScreen() {
  return (
    <div class="border-1 border-[#f90] p-6 flex grow flex-col boot-menu">
      <div class=" text-sm mb-4">⚠ NO BUNDLE LOADED</div>
      <p class="mb-6 ">Please provide a Tonk bundle to continue:</p>
      <ul class="list-none space-y-3 pl-6">
        <li class="">→ Drag a .tonk file onto this window</li>
        <li class="">
          → Or add URL parameter:{' '}
          <code class="bg-[#333] px-2 py-1 rounded">?bundle=&lt;url&gt;</code>
        </li>
      </ul>
      <p class="mt-8 text-[#999] text-xs">
        Example:{' '}
        <code class="bg-[#333] px-2 py-1 rounded">
          ?bundle=http://localhost:8081/.manifest.tonk
        </code>
      </p>
    </div>
  );
}
