import React from 'react';
import { WidgetDefinition } from '../../index';
import PomodoroTimerComponent from './component';

const widgetDefinition: WidgetDefinition = {
  id: 'pomodoro-timer',
  name: 'Pomodoro Timer',
  description: 'A complete Pomodoro Timer widget with customizable work/break intervals, session tracking, and notifications for enhanced productivity',
  component: PomodoroTimerComponent,
  defaultProps: {
    width: 300,
    height: 450,
  },
  icon: '🍅',
  category: 'productivity',
};

export default widgetDefinition;