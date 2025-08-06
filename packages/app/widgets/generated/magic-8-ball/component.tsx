import React, { useState } from 'react';
import BaseWidget from '../../templates/BaseWidget';
import { WidgetProps } from '../../index';

interface Magic8BallProps extends WidgetProps {}

const responses = [
  "It is certain",
  "Without a doubt",
  "Yes definitely",
  "You may rely on it",
  "As I see it, yes",
  "Most likely",
  "Outlook good",
  "Yes",
  "Signs point to yes",
  "Reply hazy, try again",
  "Ask again later",
  "Better not tell you now",
  "Cannot predict now",
  "Concentrate and ask again",
  "Don't count on it",
  "My reply is no",
  "My sources say no",
  "Outlook not so good",
  "Very doubtful",
  "Absolutely not"
];

const Magic8Ball: React.FC<Magic8BallProps> = (props) => {
  const [response, setResponse] = useState<string>("Ask a question and shake");
  const [isShaking, setIsShaking] = useState<boolean>(false);

  const getRandomResponse = () => {
    const randomIndex = Math.floor(Math.random() * responses.length);
    return responses[randomIndex];
  };

  const handleShake = () => {
    if (isShaking) return;

    setIsShaking(true);
    setTimeout(() => {
      setResponse(getRandomResponse());
      setIsShaking(false);
    }, 1000);
  };

  return (
    <BaseWidget {...props}>
      <div className="flex flex-col items-center justify-center h-full p-4 bg-gradient-to-b from-indigo-900 to-purple-900 rounded-lg">
        <div 
          className={`relative w-32 h-32 mb-6 cursor-pointer transition-transform duration-200 ${
            isShaking ? 'animate-pulse scale-110' : 'hover:scale-105'
          }`}
          onClick={handleShake}
        >
          <div className="w-full h-full bg-black rounded-full flex items-center justify-center shadow-2xl">
            <div className="w-16 h-16 bg-white rounded-full flex items-center justify-center shadow-inner">
              <span className="text-2xl">8</span>
            </div>
          </div>
        </div>
        
        <div className="text-center space-y-4">
          <h3 className="text-white font-bold text-lg">Magic 8-Ball</h3>
          
          <div className="bg-white/10 backdrop-blur-sm rounded-lg p-4 min-h-[60px] flex items-center justify-center">
            <p className="text-white text-sm text-center font-medium">
              {isShaking ? "🔮 Shaking..." : response}
            </p>
          </div>
          
          <button
            onClick={handleShake}
            disabled={isShaking}
            className="bg-purple-600 hover:bg-purple-700 disabled:bg-purple-800 disabled:cursor-not-allowed text-white font-bold py-2 px-4 rounded-lg transition-colors duration-200 shadow-lg"
          >
            {isShaking ? "Shaking..." : "Shake Again"}
          </button>
        </div>
        
        <div className="text-white/60 text-xs mt-4 text-center">
          Click the ball or button to shake
        </div>
      </div>
    </BaseWidget>
  );
};

export default Magic8Ball;