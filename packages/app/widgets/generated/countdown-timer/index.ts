import React from 'react';
import { WidgetDefinition } from '../../index';
import CountdownTimerComponent from './component';

const widgetDefinition: WidgetDefinition = {
  id: 'countdown-timer',
  name: 'Countdown Timer',
  description: 'A customizable countdown timer widget that counts down to a specific date and time',
  component: CountdownTimerComponent,
  defaultProps: {
    width: 280,
    height: 220,
  },
  icon: '⏰',
  category: 'productivity',
};

export default widgetDefinition;