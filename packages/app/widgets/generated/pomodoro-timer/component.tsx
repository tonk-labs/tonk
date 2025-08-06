import React, { useState, useEffect, useRef } from 'react';
import { WidgetProps } from '../../index';
import BaseWidget from '../../templates/BaseWidget';

interface PomodoroTimerWidgetProps extends WidgetProps {
  workDuration?: number;
  shortBreakDuration?: number;
  longBreakDuration?: number;
  sessionsUntilLongBreak?: number;
}

type TimerState = 'stopped' | 'running' | 'paused';
type SessionType = 'work' | 'shortBreak' | 'longBreak';

interface Session {
  type: SessionType;
  duration: number;
  completedAt?: Date;
}

const PomodoroTimer: React.FC<PomodoroTimerWidgetProps> = ({
  id,
  x,
  y,
  width = 300,
  height = 450,
  onMove,
  selected = false,
  workDuration = 25,
  shortBreakDuration = 5,
  longBreakDuration = 15,
  sessionsUntilLongBreak = 4,
}) => {
  const [timeLeft, setTimeLeft] = useState(workDuration * 60);
  const [timerState, setTimerState] = useState<TimerState>('stopped');
  const [currentSession, setCurrentSession] = useState<SessionType>('work');
  const [completedSessions, setCompletedSessions] = useState<Session[]>([]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [autoStartNext, setAutoStartNext] = useState(true);
  const [enableSound, setEnableSound] = useState(true);
  
  // Settings state
  const [settings, setSettings] = useState({
    workDuration,
    shortBreakDuration,
    longBreakDuration,
    sessionsUntilLongBreak,
  });

  const intervalRef = useRef<NodeJS.Timeout | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);

  useEffect(() => {
    // Create audio element for notifications
    audioRef.current = new Audio('data:audio/wav;base64,UklGRl9vT19XQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YU');
  }, []);

  useEffect(() => {
    if (timerState === 'running' && timeLeft > 0) {
      intervalRef.current = setInterval(() => {
        setTimeLeft(prev => prev - 1);
      }, 1000);
    } else {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    }

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    };
  }, [timerState, timeLeft]);

  useEffect(() => {
    if (timeLeft === 0) {
      handleTimerComplete();
    }
  }, [timeLeft]);

  const playNotificationSound = () => {
    if (enableSound && audioRef.current) {
      audioRef.current.play().catch(e => console.log('Audio play failed:', e));
    }
  };

  const handleTimerComplete = () => {
    playNotificationSound();
    
    if (currentSession === 'work') {
      const newSession: Session = {
        type: 'work',
        duration: settings.workDuration,
        completedAt: new Date(),
      };
      setCompletedSessions(prev => [...prev, newSession]);
    }

    // Determine next session type
    let nextSession: SessionType;
    let nextDuration: number;

    if (currentSession === 'work') {
      const completedWorkSessions = completedSessions.filter(s => s.type === 'work').length + 1;
      if (completedWorkSessions % settings.sessionsUntilLongBreak === 0) {
        nextSession = 'longBreak';
        nextDuration = settings.longBreakDuration;
      } else {
        nextSession = 'shortBreak';
        nextDuration = settings.shortBreakDuration;
      }
    } else {
      nextSession = 'work';
      nextDuration = settings.workDuration;
    }

    setCurrentSession(nextSession);
    setTimeLeft(nextDuration * 60);
    setTimerState(autoStartNext ? 'running' : 'stopped');
  };

  const startTimer = () => {
    setTimerState('running');
  };

  const pauseTimer = () => {
    setTimerState('paused');
  };

  const resetTimer = () => {
    setTimerState('stopped');
    const currentDuration = currentSession === 'work' 
      ? settings.workDuration 
      : currentSession === 'shortBreak' 
        ? settings.shortBreakDuration 
        : settings.longBreakDuration;
    setTimeLeft(currentDuration * 60);
  };

  const skipSession = () => {
    setTimerState('stopped');
    handleTimerComplete();
  };

  const formatTime = (seconds: number) => {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
  };

  const updateSettings = () => {
    setTimeLeft(settings[currentSession === 'work' ? 'workDuration' : 
               currentSession === 'shortBreak' ? 'shortBreakDuration' : 
               'longBreakDuration'] * 60);
    setSettingsOpen(false);
  };

  const resetAll = () => {
    setCompletedSessions([]);
    setCurrentSession('work');
    setTimeLeft(settings.workDuration * 60);
    setTimerState('stopped');
  };

  const getSessionColor = () => {
    switch (currentSession) {
      case 'work': return 'text-red-500 border-red-500';
      case 'shortBreak': return 'text-green-500 border-green-500';
      case 'longBreak': return 'text-blue-500 border-blue-500';
      default: return 'text-gray-500 border-gray-500';
    }
  };

  const getSessionEmoji = () => {
    switch (currentSession) {
      case 'work': return '💼';
      case 'shortBreak': return '☕';
      case 'longBreak': return '🌴';
      default: return '⏱️';
    }
  };

  return (
    <BaseWidget
      id={id}
      x={x}
      y={y}
      width={width}
      height={height}
      onMove={onMove}
      selected={selected}
      title="Pomodoro Timer"
      backgroundColor="bg-white"
      borderColor={getSessionColor()}
    >
      <div className="flex flex-col h-full p-4">
        {/* Header */}
        <div className="flex justify-between items-center mb-4">
          <div className="flex items-center gap-2">
            <span className="text-2xl">{getSessionEmoji()}</span>
            <span className="text-sm font-medium text-gray-700 capitalize">
              {currentSession.replace(/([A-Z])/g, ' $1').trim()}
            </span>
          </div>
          <button
            onClick={() => setSettingsOpen(!settingsOpen)}
            className="text-gray-500 hover:text-gray-700 text-sm"
          >
            ⚙️
          </button>
        </div>

        {/* Timer Display */}
        <div className="text-center mb-6">
          <div className={`text-5xl font-bold ${getSessionColor()}`}>
            {formatTime(timeLeft)}
          </div>
          <div className="text-sm text-gray-600 mt-2">
            {completedSessions.filter(s => s.type === 'work').length} sessions completed
          </div>
        </div>

        {/* Progress Ring */}
        <div className="relative w-32 h-32 mx-auto mb-6">
          <svg className="w-full h-full" viewBox="0 0 36 36">
            <path
              className="text-gray-200"
              stroke="currentColor"
              strokeWidth="3"
              fill="none"
              d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
            />
            <path
              className={getSessionColor()}
              stroke="currentColor"
              strokeWidth="3"
              strokeLinecap="round"
              fill="none"
              strokeDasharray={`${((settings[currentSession === 'work' ? 'workDuration' : 
                currentSession === 'shortBreak' ? 'shortBreakDuration' : 
                'longBreakDuration'] * 60 - timeLeft) / 
                (settings[currentSession === 'work' ? 'workDuration' : 
                currentSession === 'shortBreak' ? 'shortBreakDuration' : 
                'longBreakDuration'] * 60)) * 100}, 100`}
              d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831"
            />
          </svg>
          <div className="absolute inset-0 flex items-center justify-center">
            <span className="text-lg font-semibold text-gray-700">
              {Math.round(((settings[currentSession === 'work' ? 'workDuration' : 
                currentSession === 'shortBreak' ? 'shortBreakDuration' : 
                'longBreakDuration'] * 60 - timeLeft) / 
                (settings[currentSession === 'work' ? 'workDuration' : 
                currentSession === 'shortBreak' ? 'shortBreakDuration' : 
                'longBreakDuration'] * 60)) * 100)}%
            </span>
          </div>
        </div>

        {/* Controls */}
        <div className="flex justify-center gap-3 mb-6">
          {timerState === 'stopped' && (
            <button
              onClick={startTimer}
              className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
            >
              Start
            </button>
          )}
          {timerState === 'running' && (
            <button
              onClick={pauseTimer}
              className="px-6 py-2 bg-yellow-500 text-white rounded-lg hover:bg-yellow-600 transition-colors"
            >
              Pause
            </button>
          )}
          {timerState === 'paused' && (
            <button
              onClick={startTimer}
              className="px-6 py-2 bg-green-500 text-white rounded-lg hover:bg-green-600 transition-colors"
            >
              Resume
            </button>
          )}
          <button
            onClick={resetTimer}
            className="px-4 py-2 bg-gray-300 text-gray-700 rounded-lg hover:bg-gray-400 transition-colors"
          >
            Reset
          </button>
          <button
            onClick={skipSession}
            className="px-4 py-2 bg-purple-500 text-white rounded-lg hover:bg-purple-600 transition-colors"
          >
            Skip
          </button>
        </div>

        {/* Settings Modal */}
        {settingsOpen && (
          <div className="absolute inset-0 bg-white rounded-lg p-4 overflow-y-auto">
            <div className="flex justify-between items-center mb-4">
              <h3 className="text-lg font-semibold">Settings</h3>
              <button
                onClick={() => setSettingsOpen(false)}
                className="text-gray-500 hover:text-gray-700"
              >
                ✕
              </button>
            </div>

            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Work Duration (minutes)
                </label>
                <input
                  type="number"
                  value={settings.workDuration}
                  onChange={(e) => setSettings(prev => ({ ...prev, workDuration: parseInt(e.target.value) || 25 }))}
                  className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                  min="1"
                  max="60"
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Short Break (minutes)
                </label>
                <input
                  type="number"
                  value={settings.shortBreakDuration}
                  onChange={(e) => setSettings(prev => ({ ...prev, shortBreakDuration: parseInt(e.target.value) || 5 }))}
                  className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                  min="1"
                  max="30"
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Long Break (minutes)
                </label>
                <input
                  type="number"
                  value={settings.longBreakDuration}
                  onChange={(e) => setSettings(prev => ({ ...prev, longBreakDuration: parseInt(e.target.value) || 15 }))}
                  className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                  min="1"
                  max="60"
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-700 mb-1">
                  Sessions until Long Break
                </label>
                <input
                  type="number"
                  value={settings.sessionsUntilLongBreak}
                  onChange={(e) => setSettings(prev => ({ ...prev, sessionsUntilLongBreak: parseInt(e.target.value) || 4 }))}
                  className="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                  min="1"
                  max="10"
                />
              </div>

              <div className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={autoStartNext}
                  onChange={(e) => setAutoStartNext(e.target.checked)}
                  className="rounded"
                />
                <label className="text-sm text-gray-700">Auto-start next session</label>
              </div>

              <div className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={enableSound}
                  onChange={(e) => setEnableSound(e.target.checked)}
                  className="rounded"
                />
                <label className="text-sm text-gray-700">Enable sound notifications</label>
              </div>

              <div className="flex gap-2 mt-6">
                <button
                  onClick={updateSettings}
                  className="flex-1 px-4 py-2 bg-blue-500 text-white rounded-md hover:bg-blue-600 transition-colors"
                >
                  Apply
                </button>
                <button
                  onClick={resetAll}
                  className="flex-1 px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600 transition-colors"
                >
                  Reset All
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </BaseWidget>
  );
};

export default PomodoroTimer;