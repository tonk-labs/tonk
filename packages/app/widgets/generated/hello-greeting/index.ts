import React from 'react';
import { WidgetDefinition } from '../../index';
import HelloGreetingComponent from './component';

const widgetDefinition: WidgetDefinition = {
  id: 'hello-greeting',
  name: 'Hello Greeting',
  description: 'A friendly greeting widget that displays customizable welcome messages with beautiful animations',
  component: HelloGreetingComponent,
  defaultProps: {
    width: 280,
    height: 180,
  },
  icon: '👋',
  category: 'utility',
};

export default widgetDefinition;