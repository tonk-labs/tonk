import { Router, Route, Switch, Link } from 'wouter';
import Times from './Times';
import { useEffect, useState } from 'react';
import viteLogo from '../public/vite.svg';
import { useVFS } from './hooks/useVFS';

const possibilities = [
  'may',
  'can',
  'often',
  'sometimes',
  'anytimes',
  'perhaps',
  'should',
];

function Home() {
  const [observed] = useState(
    possibilities[Math.floor(Math.random() * possibilities.length - 1)]
  );
  const [files, setFiles] = useState<string[]>([]);

  const vfs = useVFS();

  useEffect(() => {
    if (!vfs.isReady) return;
    const getFiles = async () => {
      const fileList = await vfs.vfs.listDirectory('/');
      setFiles(fileList as string[]);
    };
    getFiles();
  }, [vfs]);

  return (
    <main className="p-4 w-screen h-screen">
      <div className="flex flex-col w-full items-center justify-center my-20">
        <div className="flex text-5xl font-bold">Vite + Tonk + React</div>
        <div className="flex w-auth gap-4 my-10">
          <img
            src="tonk.png"
            className="w-24 h-24 flex rounded-full animate-spin"
          />
          <img src={viteLogo} className="w-24 h-24 flex animate-bounce" />
        </div>
      </div>
      <div className="text-3xl font-bold">
        🎉 some{' '}
        <Link href="/times" className="text-black font-bold px-2">
          <span className="underline">times</span>
        </Link>{' '}
        this {observed} work(s)
        {
          files.length > 0 && (
            <>
              <br />
              <br />
              📁 files:{' '}
              {files.map((file, index) => (
                <span key={index}>{file}</span>
              ))}
            </>
          )
        }
      </div>
    </main>
  );
}

function App() {
  return (
    <Router base="/bundling">
      <Switch>
        <Route path="/" component={Home} />
        <Route path="/times" component={Times} />
        <Route>404: Page not found</Route>
      </Switch>
    </Router>
  );
}

export default App;
