import React, { useState, useEffect } from 'react';
import BaseWidget from '../BaseWidget';
import { WidgetProps } from '../index';

interface CountdownTimerProps extends WidgetProps {}

interface TimeLeft {
  days: number;
  hours: number;
  minutes: number;
  seconds: number;
}

const CountdownTimer: React.FC<CountdownTimerProps> = (props) => {
  const [targetDate, setTargetDate] = useState<string>(() => {
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    tomorrow.setHours(12, 0, 0, 0);
    return tomorrow.toISOString().slice(0, 16);
  });
  
  const [timeLeft, setTimeLeft] = useState<TimeLeft>({ days: 0, hours: 0, minutes: 0, seconds: 0 });
  const [isEditing, setIsEditing] = useState(false);
  const [tempDate, setTempDate] = useState(targetDate);
  const [isComplete, setIsComplete] = useState(false);

  const calculateTimeLeft = () => {
    const difference = +new Date(targetDate) - +new Date();
    
    if (difference > 0) {
      setTimeLeft({
        days: Math.floor(difference / (1000 * 60 * 60 * 24)),
        hours: Math.floor((difference / (1000 * 60 * 60)) % 24),
        minutes: Math.floor((difference / 1000 / 60) % 60),
        seconds: Math.floor((difference / 1000) % 60),
      });
      setIsComplete(false);
    } else {
      setTimeLeft({ days: 0, hours: 0, minutes: 0, seconds: 0 });
      setIsComplete(true);
    }
  };

  useEffect(() => {
    calculateTimeLeft();
    const timer = setInterval(calculateTimeLeft, 1000);

    return () => clearInterval(timer);
  }, [targetDate]);

  const handleSaveDate = () => {
    setTargetDate(tempDate);
    setIsEditing(false);
    setIsComplete(false);
  };

  const handleReset = () => {
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    tomorrow.setHours(12, 0, 0, 0);
    const newDate = tomorrow.toISOString().slice(0, 16);
    setTargetDate(newDate);
    setTempDate(newDate);
    setIsComplete(false);
  };

  const formatUnit = (unit: number, label: string) => (
    <div className="flex flex-col items-center">
      <div className="text-2xl font-bold text-gray-800 dark:text-gray-100">
        {unit.toString().padStart(2, '0')}
      </div>
      <div className="text-xs text-gray-500 dark:text-gray-400 uppercase tracking-wide">
        {label}
      </div>
    </div>
  );

  return (
    <BaseWidget 
      id={props.id}
      x={props.x}
      y={props.y}
      width={props.width || 280}
      height={props.height || 220}
      onMove={props.onMove}
      selected={props.selected}
      backgroundColor="bg-white dark:bg-gray-800"
      borderColor="border-gray-200 dark:border-gray-600"
      title="⏰ Countdown Timer"
    >
      <div className="flex flex-col h-full p-4">
        {isEditing ? (
          <div className="flex flex-col space-y-3">
            <label className="text-sm font-medium text-gray-700 dark:text-gray-300">
              Set Target Date & Time:
            </label>
            <input
              type="datetime-local"
              value={tempDate}
              onChange={(e) => setTempDate(e.target.value)}
              className="px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-md bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100"
            />
            <div className="flex space-x-2">
              <button
                onClick={handleSaveDate}
                className="flex-1 px-3 py-2 bg-blue-500 text-white rounded-md hover:bg-blue-600 text-sm"
              >
                Save
              </button>
              <button
                onClick={() => {
                  setIsEditing(false);
                  setTempDate(targetDate);
                }}
                className="flex-1 px-3 py-2 bg-gray-300 text-gray-700 rounded-md hover:bg-gray-400 text-sm"
              >
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <div className="flex flex-col h-full">
            <div className="flex-1 flex items-center justify-center">
              {isComplete ? (
                <div className="text-center">
                  <div className="text-4xl mb-2">🎉</div>
                  <div className="text-lg font-semibold text-green-600 dark:text-green-400">
                    Time's Up!
                  </div>
                  <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                    Target reached
                  </div>
                </div>
              ) : (
                <div className="flex space-x-3">
                  {formatUnit(timeLeft.days, 'Days')}
                  <div className="text-2xl font-bold text-gray-400 dark:text-gray-500">:</div>
                  {formatUnit(timeLeft.hours, 'Hours')}
                  <div className="text-2xl font-bold text-gray-400 dark:text-gray-500">:</div>
                  {formatUnit(timeLeft.minutes, 'Min')}
                  <div className="text-2xl font-bold text-gray-400 dark:text-gray-500">:</div>
                  {formatUnit(timeLeft.seconds, 'Sec')}
                </div>
              )}
            </div>
            
            <div className="flex space-x-2 mt-4">
              <button
                onClick={() => setIsEditing(true)}
                className="flex-1 px-3 py-2 bg-blue-500 text-white rounded-md hover:bg-blue-600 text-sm"
              >
                Set Date
              </button>
              <button
                onClick={handleReset}
                className="flex-1 px-3 py-2 bg-gray-300 dark:bg-gray-600 text-gray-700 dark:text-gray-200 rounded-md hover:bg-gray-400 dark:hover:bg-gray-500 text-sm"
              >
                Reset
              </button>
            </div>
          </div>
        )}
      </div>
    </BaseWidget>
  );
};

export default CountdownTimer;