import React from 'react';
import { WidgetDefinition } from '../../index';
import DiceRollerComponent from './component';

const widgetDefinition: WidgetDefinition = {
  id: 'dice-roller',
  name: 'Dice Roller',
  description: 'A simple digital dice roller that randomly generates numbers from 1-6',
  component: DiceRollerComponent,
  defaultProps: {
    width: 200,
    height: 180,
  },
  icon: '🎲',
  category: 'entertainment',
};

export default widgetDefinition;