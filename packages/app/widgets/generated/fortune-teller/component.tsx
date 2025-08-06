import React, { useState, useEffect } from 'react';
import { WidgetProps } from '../../index';
import BaseWidget from '../../templates/BaseWidget';

interface FortuneTellerProps extends WidgetProps {
  // Add custom props here
}

interface Fortune {
  text: string;
  category: 'love' | 'career' | 'wisdom' | 'general';
  intensity: 'mild' | 'moderate' | 'intense';
}

const fortunes: Fortune[] = [
  // Love fortunes
  { text: "Someone from your past will reappear with transformative news.", category: 'love', intensity: 'moderate' },
  { text: "A chance encounter under the moonlight will change everything.", category: 'love', intensity: 'intense' },
  { text: "Your heart's desire is closer than you think - look to your left.", category: 'love', intensity: 'mild' },
  
  // Career fortunes
  { text: "An unexpected email will open doors you didn't know existed.", category: 'career', intensity: 'moderate' },
  { text: "Your greatest strength lies in what others consider your weakness.", category: 'career', intensity: 'mild' },
  { text: "A golden opportunity disguised as extra work approaches.", category: 'career', intensity: 'intense' },
  
  // Wisdom fortunes
  { text: "The answer you seek is hidden in your childhood dreams.", category: 'wisdom', intensity: 'mild' },
  { text: "Let go of what you think you know, and truth will find you.", category: 'wisdom', intensity: 'intense' },
  { text: "Three small steps today become a giant leap tomorrow.", category: 'wisdom', intensity: 'moderate' },
  
  // General fortunes
  { text: "A stranger's smile will bring unexpected fortune this week.", category: 'general', intensity: 'mild' },
  { text: "The color blue will play a significant role in your near future.", category: 'general', intensity: 'moderate' },
  { text: "Your intuition whispers truth - learn to trust it completely.", category: 'general', intensity: 'intense' },
];

const FortuneTeller: React.FC<FortuneTellerProps> = ({
  id,
  x,
  y,
  width = 320,
  height = 400,
  selected,
  onMove,
  data,
}) => {
  const [currentFortune, setCurrentFortune] = useState<Fortune | null>(null);
  const [isRevealing, setIsRevealing] = useState(false);
  const [selectedCategory, setSelectedCategory] = useState<'all' | 'love' | 'career' | 'wisdom' | 'general'>('all');
  const [fortuneCount, setFortuneCount] = useState(0);

  useEffect(() => {
    // Initialize with a welcome message
    setCurrentFortune({
      text: "Welcome, seeker. The crystal ball awaits your touch...",
      category: 'general',
      intensity: 'mild'
    });
  }, []);

  const getFilteredFortunes = () => {
    if (selectedCategory === 'all') return fortunes;
    return fortunes.filter(f => f.category === selectedCategory);
  };

  const revealFortune = () => {
    setIsRevealing(true);
    setTimeout(() => {
      const filtered = getFilteredFortunes();
      const randomIndex = Math.floor(Math.random() * filtered.length);
      const newFortune = filtered[randomIndex];
      
      setCurrentFortune(newFortune);
      setFortuneCount(prev => prev + 1);
      setIsRevealing(false);
    }, 2000);
  };

  const categoryColors = {
    love: 'text-pink-500',
    career: 'text-blue-500',
    wisdom: 'text-purple-500',
    general: 'text-gray-500'
  };

  const getIntensityIcon = (intensity: string) => {
    switch (intensity) {
      case 'mild': return '✨';
      case 'moderate': return '🌟';
      case 'intense': return '💫';
      default: return '⭐';
    }
  };

  return (
    <BaseWidget
      id={id}
      x={x}
      y={y}
      width={width}
      height={height}
      selected={selected}
      onMove={onMove}
      title="Mystic Fortune Teller"
      backgroundColor="bg-gradient-to-br from-purple-900 via-indigo-900 to-purple-800"
      borderColor="border-purple-400"
    >
      <div className="h-full flex flex-col">
        {/* Crystal Ball */}
        <div className="flex justify-center mt-4">
          <div className={`relative w-32 h-32 rounded-full bg-gradient-radial from-purple-300 via-purple-400 to-purple-600 shadow-2xl border-4 border-purple-300 transition-all duration-1000 ${isRevealing ? 'animate-pulse' : ''}`}>
            <div className="absolute inset-4 rounded-full bg-gradient-radial from-white/30 to-purple-300/50 animate-pulse" />
            <div className="absolute inset-0 rounded-full opacity-50" style={{
              background: 'radial-gradient(circle at 30% 30%, white, transparent 40%)'
            }} />
            {isRevealing && (
              <div className="absolute inset-0 flex items-center justify-center">
                <div className="w-16 h-16 rounded-full bg-purple-200 animate-bounce" />
              </div>
            )}
          </div>
        </div>

        {/* Fortune Display */}
        <div className="flex-1 px-4 py-3">
          <div className="bg-black/20 rounded-lg p-4 backdrop-blur-sm border border-purple-400/30 min-h-24">
            {currentFortune && (
              <div className="text-center">
                <div className={`text-lg mb-2 ${categoryColors[currentFortune.category]}`}>
                  {getIntensityIcon(currentFortune.intensity)} {currentFortune.category.toUpperCase()}
                </div>
                <p className="text-white/90 text-sm leading-relaxed italic">
                  "{currentFortune.text}"
                </p>
              </div>
            )}
          </div>
        </div>

        {/* Category Selector */}
        <div className="px-4 pb-2">
          <div className="flex justify-center space-x-1 mb-3">
            {(['all', 'love', 'career', 'wisdom', 'general'] as const).map(cat => (
              <button
                key={cat}
                onClick={() => setSelectedCategory(cat)}
                className={`px-2 py-1 text-xs rounded transition-colors ${
                  selectedCategory === cat
                    ? 'bg-purple-500 text-white'
                    : 'bg-purple-800/50 text-purple-200 hover:bg-purple-700/50'
                }`}
              >
                {cat === 'all' ? '✨' : cat === 'love' ? '💖' : cat === 'career' ? '💼' : cat === 'wisdom' ? '🧙‍♀️' : '🔮'}
              </button>
            ))}
          </div>
        </div>

        {/* Reveal Button */}
        <div className="px-4 pb-4">
          <button
            onClick={revealFortune}
            disabled={isRevealing}
            className="w-full py-3 bg-gradient-to-r from-purple-500 to-pink-500 text-white rounded-lg font-semibold hover:from-purple-600 hover:to-pink-600 transition-all duration-300 transform hover:scale-105 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed shadow-lg"
          >
            {isRevealing ? (
              <span className="flex items-center justify-center">
                <span className="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin mr-2" />
                Consulting the spirits...
              </span>
            ) : (
              "🔮 Reveal My Fortune 🔮"
            )}
          </button>
        </div>

        {/* Fortune Counter */}
        <div className="text-center pb-2">
          <span className="text-xs text-purple-200/70">
            {fortuneCount} fortune{fortuneCount !== 1 ? 's' : ''} revealed
          </span>
        </div>
      </div>
    </BaseWidget>
  );
};

export default FortuneTeller;