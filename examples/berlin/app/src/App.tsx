import Layout from './components/layout/layout';
import { Editor } from './features/editor';
import { PresenceIndicators, usePresenceTracking } from './features/presence';

function App() {
  // Enable presence tracking
  usePresenceTracking();

  return (
    <Layout>
      <PresenceIndicators className="fixed top-4 right-4 z-50" maxVisible={5} />
      <Editor/>
    </Layout>
  );
}

export default App;
