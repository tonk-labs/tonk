import React, { useState, useEffect } from 'react';
import { WidgetProps } from '../index';
import BaseWidget from '../BaseWidget';

interface HelloGreetingProps extends WidgetProps {
  // Custom props for this widget
  userName?: string;
  greetingStyle?: 'friendly' | 'formal' | 'fun';
}

const HelloGreeting: React.FC<HelloGreetingProps> = ({
  id,
  x,
  y,
  selected,
  onMove,
  userName = 'Friend',
  greetingStyle = 'friendly',
}) => {
  const [currentTime, setCurrentTime] = useState(new Date());
  const [showEmoji, setShowEmoji] = useState(false);

  // Update time every minute
  useEffect(() => {
    const timer = setInterval(() => {
      setCurrentTime(new Date());
    }, 60000);

    return () => clearInterval(timer);
  }, []);

  // Emoji animation trigger
  useEffect(() => {
    setShowEmoji(true);
  }, []);

  const getGreeting = () => {
    const hour = currentTime.getHours();
    
    if (greetingStyle === 'formal') {
      if (hour < 12) return 'Good morning';
      if (hour < 17) return 'Good afternoon';
      return 'Good evening';
    }
    
    if (greetingStyle === 'fun') {
      if (hour < 12) return 'Rise and shine';
      if (hour < 14) return 'Happy lunch time';
      if (hour < 17) return 'Afternoon adventures';
      if (hour < 20) return 'Golden hour vibes';
      return 'Sweet dreams ahead';
    }
    
    // Friendly style
    if (hour < 12) return 'Good morning';
    if (hour < 17) return 'Hey there';
    return 'Good evening';
  };

  const getEmoji = () => {
    const hour = currentTime.getHours();
    
    if (hour < 12) return '☀️';
    if (hour < 17) return '🌤️';
    if (hour < 20) return '🌅';
    return '🌙';
  };

  return (
    <BaseWidget
      id={id}
      x={x}
      y={y}
      width={280}
      height={180}
      selected={selected}
      onMove={onMove}
      title="Hello"
      backgroundColor="bg-gradient-to-br from-purple-50 to-pink-50"
      borderColor="border-purple-200"
    >
      <div className="p-6 flex flex-col items-center justify-center h-full text-center">
        {/* Animated Emoji */}
        <div className={`text-4xl mb-3 transition-all duration-500 transform ${
          showEmoji ? 'scale-100 opacity-100' : 'scale-50 opacity-0'
        }`}>
          {getEmoji()}
        </div>
        
        {/* Greeting Text */}
        <div className="text-lg font-medium text-gray-800 mb-1">
          {getGreeting()}
        </div>
        
        {/* User Name */}
        <div className="text-2xl font-bold text-transparent bg-clip-text bg-gradient-to-r from-purple-600 to-pink-600 mb-2">
          {userName}!
        </div>
        
        {/* Time-based Sub-greeting */}
        <div className="text-sm text-gray-600 font-light">
          {currentTime.toLocaleDateString('en-US', { 
            weekday: 'long', 
            month: 'short', 
            day: 'numeric' 
          })}
        </div>
      </div>
    </BaseWidget>
  );
};

export default HelloGreeting;