import React from 'react';
import { useSimpleStore } from '../stores/simple-store';

const HelloWorld: React.FC = () => {
  const { count, increment, decrement, reset } = useSimpleStore();

  return (
    <div className="flex items-center justify-center min-h-screen bg-gradient-to-r from-blue-500 to-purple-600">
      <div className="text-center">
        <h1 className="text-6xl font-bold text-white mb-4">Hello World!</h1>
        <p className="text-xl text-white opacity-90 mb-8">
          Welcome to your Tonk application
        </p>

        <div className="bg-white/20 backdrop-blur-sm rounded-lg p-8 max-w-md mx-auto">
          <h2 className="text-2xl font-semibold text-white mb-4">Counter</h2>
          <div className="text-4xl font-bold text-white mb-6">{count}</div>

          <div className="flex gap-4 justify-center mb-4">
            <button
              onClick={decrement}
              className="bg-red-500 hover:bg-red-600 text-white font-bold py-2 px-6 rounded-lg transition-colors"
            >
              -
            </button>
            <button
              onClick={increment}
              className="bg-green-500 hover:bg-green-600 text-white font-bold py-2 px-6 rounded-lg transition-colors"
            >
              +
            </button>
          </div>

          <button
            onClick={reset}
            className="bg-gray-500 hover:bg-gray-600 text-white font-bold py-2 px-4 rounded-lg transition-colors"
          >
            Reset
          </button>
        </div>
      </div>
    </div>
  );
};

export default HelloWorld;
