import React, { useState } from 'react';
import { WidgetProps } from '../../index';
import BaseWidget from '../../templates/BaseWidget';

interface DiceRollerProps extends WidgetProps {}

const DiceRoller: React.FC<DiceRollerProps> = ({
  id,
  x,
  y,
  width = 200,
  height = 180,
  selected,
  onMove,
  data,
}) => {
  const [diceValue, setDiceValue] = useState<number>(6);
  const [isRolling, setIsRolling] = useState(false);

  const rollDice = () => {
    setIsRolling(true);
    
    // Add a small delay for the "rolling" animation effect
    setTimeout(() => {
      const newValue = Math.floor(Math.random() * 6) + 1;
      setDiceValue(newValue);
      setIsRolling(false);
    }, 300);
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
      title="Dice Roller"
      backgroundColor="bg-white"
      borderColor="border-gray-200"
    >
      <div className="p-4 flex flex-col items-center justify-center h-full">
        <div className={`text-6xl mb-4 transition-transform duration-300 ${
          isRolling ? 'animate-bounce' : ''
        }`}>
          {diceValue === 1 && '🎲'}
          {diceValue === 2 && '🎲'}
          {diceValue === 3 && '🎲'}
          {diceValue === 4 && '🎲'}
          {diceValue === 5 && '🎲'}
          {diceValue === 6 && '🎲'}
        </div>
        
        <div className="text-3xl font-bold text-gray-800 mb-4">
          {diceValue}
        </div>
        
        <button
          onClick={rollDice}
          disabled={isRolling}
          className="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 disabled:bg-gray-400 disabled:cursor-not-allowed transition-colors duration-200"
        >
          {isRolling ? 'Rolling...' : 'Roll'}
        </button>
      </div>
    </BaseWidget>
  );
};

export default DiceRoller;