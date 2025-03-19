import {Command} from 'commander';
import chalk from 'chalk';

export const helloCommand = new Command('hello')
  .description('Display a welcome message for Tonk')
  .action(() => {
    console.log(`
・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜

       _____   U  ___ u  _   _       _  __    
      |_ " _|   \/"_ \/ | \ |"|     |"|/ /    
        | |     | | | |<|  \| |>    | ' /     
       /| |\.-,_| |_| |U| |\  |u  U/| . \\u   
      u |_|U \_)-\___/  |_| \_|     |_|\_\    
      _// \\_     \\    ||   \\,-.,-,>> \\,-. 
     (__) (__)   (__)   (_")  (_/  \.)   (_/  

・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜

Welcome to Tonk! 

We're excited to work with you in building your own ☆𝐿𝒾𝓉𝓉𝓁𝑒 𝐼𝓃𝓉𝑒𝓇𝓃𝑒𝓉☆.

To get started...

1. Create your first app by running the command: ${chalk.cyan('tonk create [app name]')}. 
2. Start the app with: ${chalk.cyan('npm run dev')}
3. Open in your favorite code editor and vibe.

You can find more information and how to reach us at ${chalk.blue('https://tonk.xyz')}. Feel free to ask for help in the Telegram chat or just say hello.

Cheers!
`);
    process.exit(0);
  });
