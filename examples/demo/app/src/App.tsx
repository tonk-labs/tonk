import './App.css';
import { PixelCanvas } from './components/PixelCanvas';

function App() {
  return (
    <main style={{ width: '100%', height: '100vh', overflow: 'hidden' }}>
      <PixelCanvas />
    </main>
  );
}

export default App;
