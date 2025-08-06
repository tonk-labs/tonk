import React from 'react';
import { WidgetDefinition } from '../../index';
import JokeGeneratorComponent from './component';

const widgetDefinition: WidgetDefinition = {
  id: 'joke-generator',
  name: 'Joke Generator',
  description: 'A fun widget that generates random jokes from different categories with sharing capabilities',
  component: JokeGeneratorComponent,
  defaultProps: {
    width: 350,
    height: 280,
  },
  icon: '😄',
  category: 'entertainment',
};

export default widgetDefinition;