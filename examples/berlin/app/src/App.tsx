import Layout from './components/layout/layout';
import { Editor } from './features/editor';
import { usePresenceTracking } from './features/presence';

function App() {
  // Enable presence tracking
  usePresenceTracking();

  return (
    <Layout>
      <Editor/>
    </Layout>
  );
}

export default App;
