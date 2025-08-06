import React from 'react';
import { WidgetDefinition } from '../../index';
import Magic8BallComponent from './component';

const widgetDefinition: WidgetDefinition = {
  id: 'magic-8-ball',
  name: 'Magic 8-Ball',
  description: 'Shake the mystical Magic 8-Ball to get answers to your burning questions!',
  component: Magic8BallComponent,
  defaultProps: {
    width: 280,
    height: 320,
  },
  icon: '🔮',
  category: 'entertainment',
};

export default widgetDefinition;