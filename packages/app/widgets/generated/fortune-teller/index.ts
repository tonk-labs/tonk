import React from 'react';
import { WidgetDefinition } from '../../index';
import FortuneTellerComponent from './component';

const widgetDefinition: WidgetDefinition = {
  id: 'fortune-teller',
  name: 'Fortune Teller',
  description: 'A mystical fortune teller widget that provides personalized predictions and insights based on different categories like love, career, and wisdom.',
  component: FortuneTellerComponent,
  defaultProps: {
    width: 320,
    height: 400,
  },
  icon: '🔮',
  category: 'entertainment',
};

export default widgetDefinition;