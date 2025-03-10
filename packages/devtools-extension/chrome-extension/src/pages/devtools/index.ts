import Browser from 'webextension-polyfill';

Browser
  .devtools
  .panels
  .create('Tinyfoot', 'icon-32.png', 'src/pages/panel/index.html')
  .catch(console.error);
