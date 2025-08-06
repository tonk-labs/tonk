import React, { useState, useEffect } from 'react';
import BaseWidget from '../../templates/BaseWidget';
import { WidgetProps } from '../../index';

interface Joke {
  id: number;
  category: string;
  type: string;
  setup?: string;
  delivery?: string;
  joke?: string;
}

const JokeGenerator: React.FC<WidgetProps> = (props) => {
  const [joke, setJoke] = useState<Joke | null>(null);
  const [category, setCategory] = useState<string>('Any');
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [favorites, setFavorites] = useState<Joke[]>([]);

  const categories = [
    'Any',
    'Programming',
    'Misc',
    'Dark',
    'Pun',
    'Spooky',
    'Christmas'
  ];

  const fetchJoke = async () => {
    setIsLoading(true);
    try {
      const response = await fetch(
        `https://v2.jokeapi.dev/joke/${category}?type=twopart&amount=1`
      );
      const data = await response.json();
      if (data.error === false) {
        setJoke(data);
      }
    } catch (error) {
      // Fallback joke if API fails
      const fallbackJoke: Joke = {
        id: Math.random(),
        category: 'General',
        type: 'twopart',
        setup: 'Why don\'t scientists trust atoms?',
        delivery: 'Because they make up everything!'
      };
      setJoke(fallbackJoke);
    }
    setIsLoading(false);
  };

  const toggleFavorite = (jokeToFavorite: Joke) => {
    const isFavorited = favorites.some(fav => fav.id === jokeToFavorite.id);
    if (isFavorited) {
      setFavorites(favorites.filter(fav => fav.id !== jokeToFavorite.id));
    } else {
      setFavorites([...favorites, jokeToFavorite]);
    }
  };

  const copyJoke = () => {
    if (!joke) return;
    const jokeText = joke.setup && joke.delivery 
      ? `${joke.setup}\n${joke.delivery}`
      : joke.joke || '';
    navigator.clipboard.writeText(jokeText);
  };

  useEffect(() => {
    fetchJoke();
  }, []);

  const renderJoke = () => {
    if (!joke) return null;
    
    if (joke.type === 'twopart' && joke.setup && joke.delivery) {
      return (
        <div className="space-y-3">
          <p className="text-lg font-medium text-gray-800">{joke.setup}</p>
          <p className="text-lg text-gray-600 italic">{joke.delivery}</p>
        </div>
      );
    }
    
    return <p className="text-lg text-gray-800">{joke.joke}</p>;
  };

  return (
    <BaseWidget {...props}>
      <div className="flex flex-col h-full bg-gradient-to-br from-blue-50 to-purple-50 rounded-lg p-4">
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-bold text-gray-800">😄 Joke Generator</h3>
          <div className="flex space-x-2">
            <button
              onClick={copyJoke}
              className="p-1.5 hover:bg-gray-200 rounded transition-colors"
              title="Copy joke"
            >
              📋
            </button>
            <button
              onClick={() => joke && toggleFavorite(joke)}
              className={`p-1.5 hover:bg-gray-200 rounded transition-colors ${
                joke && favorites.some(fav => fav.id === joke.id) ? 'text-red-500' : ''
              }`}
              title="Toggle favorite"
            >
              {joke && favorites.some(fav => fav.id === joke.id) ? '❤️' : '🤍'}
            </button>
          </div>
        </div>

        <div className="mb-4">
          <label className="block text-sm font-medium text-gray-700 mb-2">
            Category
          </label>
          <select
            value={category}
            onChange={(e) => setCategory(e.target.value)}
            className="w-full p-2 border border-gray-300 rounded-md focus:ring-2 focus:ring-blue-500 focus:border-transparent"
          >
            {categories.map((cat) => (
              <option key={cat} value={cat}>
                {cat}
              </option>
            ))}
          </select>
        </div>

        <div className="flex-1 flex items-center justify-center mb-4">
          <div className="w-full">
            {isLoading ? (
              <div className="flex items-center justify-center h-32">
                <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
              </div>
            ) : (
              <div className="bg-white bg-opacity-75 rounded-lg p-4 min-h-32 flex items-center justify-center">
                {renderJoke()}
              </div>
            )}
          </div>
        </div>

        <div className="flex space-x-2">
          <button
            onClick={fetchJoke}
            disabled={isLoading}
            className="flex-1 bg-blue-500 hover:bg-blue-600 disabled:bg-blue-300 text-white font-semibold py-2 px-4 rounded-lg transition-colors duration-200"
          >
            {isLoading ? 'Loading...' : 'New Joke'}
          </button>
        </div>

        <div className="mt-3 text-center">
          <span className="text-xs text-gray-500">
            {favorites.length} favorite{favorites.length !== 1 ? 's' : ''}
          </span>
        </div>
      </div>
    </BaseWidget>
  );
};

export default JokeGenerator;