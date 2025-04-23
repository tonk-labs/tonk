import React, {useState, useEffect} from 'react';
import {render, Box, Text, useInput} from 'ink';
import chalk from 'chalk';

// Original ascii art for reference (using template literals to preserve exact spacing)
const ORIGINAL_ASCII = `
       _____   U  ___ u  _   _       _  __    
      |_ " _|   \\/"_ \\/ | \\ |"|     |"|/ /    
        | |     | | | |<|  \\| |>    | ' /     
       /| |\\.-,_| |_| |U| |\\  |u  U/| . \\\\u   
      u |_|U \\_)-\\___/  |_| \\_|     |_|\\_\\    
      _// \\\\_     \\\\    ||   \\\\,-.,-,>> \\\\,-. 
     (__) (__)   (__)   (_")  (_/  \\.)   (_/  
`;

// T character frames (using template literals to preserve exact spacing)
const T_FRAMES = [
  // Frame 0 - regular T
  `       _____   
      |_ " _|   
        | |     
       /| |\\    
      u |_| U    
      _// \\\\_   
     (__) (__)  `,

  // Frame 1 - T with right arm
  `       _____   
      |_  "_|   
        | |     
       /| |--u  
      u |_|     
      _// \\\\_   
     (__) (__)  `,

  // Frame 2 - T with left arm
  `      _____      
     |_"  _|u    
       | | /     
    u--| |       
       |_|       
     _// \\\\_    
    (__)  (__)   `,
];

// O character frames (using template literals to preserve exact spacing)
const O_FRAMES = [
  // Frame 0 - regular O
  `    U  ___ u  
     \\/"_ \\/  
     | | | |  
 .-,_| |_| |
  \\_)-\\___/  
        \\\\    
       (__)   `,

  // Frame 1 - O with arms
  `        ___   
   u—— / _"\\——u
      | | | |  
      | |_| |  
   .-/,/\___/   
    \\_)   ||    
        (__)   `,

  // Frame 2 - O with stretched legs
  `      ___   
     /"_ \\  
   /| | | |\\
  u | |_| | u
     \\___/   
    _// \\\\_  
   (__) (__) `,
];

// N character frames (using template literals to preserve exact spacing)
const N_FRAMES = [
  // Frame 0 - regular N
  `    _   _     
   | \\ |"|    
  <|  \\| |>   
  U| |\\  |u  
   |_| \\_|    
   ||   \\\\,-. 
   (_")  (_/  `,

  // Frame 1 - N with different arms
  `    -   -     
   | \\ |"|    
  <|  \\| |>   
  U| |\\  |u  
   |_| \\_|    
  ,-,>> ||    
  \\.)   (_")  `,

  // Frame 2 - N with different legs
  `    _   _     
   | \\ |"|  
  <|  \\| |> 
  U| |\\  |u  
   |_| \\_|    
   //,-,>>    
  (_")\\.)     `,
];

// K character frames (using template literals to preserve exact spacing)
const K_FRAMES = [
  // Frame 0 - regular K
  `   _  __    
  |"|/ /    
  | ' /     
U/| . \\u   
  |_|\\_\\    
  ||   \\\\,-. 
  (_")  (_/  `,

  // Frame 1 - K with arm
  `   _  _     
  |"|/ /    
  | ' /     
U/| . \\—u   
  |_|\\_\\   
  ||  ||   
  (_")(_") `,

  // Frame 2 - K with legs
  `   _  __    
  |"|/ /    
  | ' /     
U/| . \\u   
  |_|\\_\\    
,-,>  \\\\,-.
 \\.)   (_/  `,

  // Frame 3 - K with all limbs
  `   _  __    
u |"|/ /    
 \\| ' /_u   
  | . \\     
  |_|\\_\\    
 _//,-,>>  
 \\.) \\.)   `,
];

// Star characters
const STAR_CHARS =
  '・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜☆。・゜☆。・゜';

// Animation configuration
const ANIMATION_CONFIG = {
  // Time between frame changes during an animation sequence (milliseconds)
  frameSpeed: 200,
  // Number of frames to show in a sequence
  sequenceLength: 3,
  // Min and max time between animation sequences (milliseconds)
  minPauseBetweenSequences: 1000,
  maxPauseBetweenSequences: 3000,
};

// Welcome message component
const WelcomeMessage = () => {
  return (
    <Box flexDirection="column" marginTop={1} width={80}>
      <Text>
        <Text bold color="green">
          Hello Builder!
        </Text>
      </Text>

      <Box marginTop={1}>
        <Text>
          Thank you for choosing Tonk! We're thrilled to have you onboard and
          excited to see what we can build together.
        </Text>
      </Box>

      <Box marginTop={2}>
        <Text bold underline color="cyan">
          Getting Started:
        </Text>
      </Box>

      <Box marginLeft={2} marginTop={1}>
        <Text>
          • Initialize a new repository:{' '}
          <Text bold color="yellow">
            tonk init
          </Text>
        </Text>
      </Box>

      <Box marginLeft={2}>
        <Text>
          • Start your sync server in the repo:{' '}
          <Text bold color="yellow">
            tonk sync
          </Text>
        </Text>
      </Box>

      <Box marginLeft={2}>
        <Text>
          • Read the documentation:{' '}
          <Text bold color="blue">
            https://tonk-labs.github.io/tonk/
          </Text>
        </Text>
      </Box>

      <Box marginTop={2}>
        <Text bold underline color="cyan">
          Community:
        </Text>
      </Box>

      <Box marginLeft={2} marginTop={1}>
        <Text>
          • Join our Telegram:{' '}
          <Text bold color="blue">
            https://t.me/+9W-4wDR9RcM2NWZk
          </Text>
        </Text>
      </Box>

      <Box marginTop={2}>
        <Text>
          Our team would love to hear from you. If you need any assistance or
          have questions, please don't hesitate to reach out through our
          community channels.
        </Text>
      </Box>

      <Box marginTop={1}>
        <Text bold color="green">
          Happy building with Tonk!
        </Text>
      </Box>
      <Box marginTop={1}>
        <Text color="yellow">(Press any key to exit)</Text>
      </Box>
    </Box>
  );
};

// Component for rendering animated ASCII art
const TonkAsciiAnimated = () => {
  // States to track current frame for each letter
  const [tFrame, setTFrame] = useState(0);
  const [oFrame, setOFrame] = useState(0);
  const [nFrame, setNFrame] = useState(0);
  const [kFrame, setKFrame] = useState(0);

  // Add state for exit handling
  const [shouldExit, setShouldExit] = useState(false);

  // Capture key input to exit
  useInput((input, key) => {
    setShouldExit(true);
  });

  // Exit effect
  useEffect(() => {
    if (shouldExit) {
      process.exit(0);
    }
  }, [shouldExit]);

  // State for star characters
  const [starIndex, setStarIndex] = useState(0);

  // Helper to get random pause duration
  const getRandomPause = () => {
    return Math.floor(
      ANIMATION_CONFIG.minPauseBetweenSequences +
        Math.random() *
          (ANIMATION_CONFIG.maxPauseBetweenSequences -
            ANIMATION_CONFIG.minPauseBetweenSequences),
    );
  };

  // Animation function for the star border
  useEffect(() => {
    const starInterval = setInterval(() => {
      setStarIndex(prev => (prev + 1) % STAR_CHARS.length);
    }, 500);

    return () => clearInterval(starInterval);
  }, []);

  // Animation function for a letter
  const animateCharacter = (
    setFrame: React.Dispatch<React.SetStateAction<number>>,
    maxFrames: number,
    animatingRef: React.MutableRefObject<boolean>,
  ) => {
    // Don't start a new animation if one is already running
    if (animatingRef.current) return;

    animatingRef.current = true;
    let frameCount = 0;

    // Start animation sequence
    const sequenceInterval = setInterval(() => {
      setFrame(prev => (prev + 1) % maxFrames);
      frameCount++;

      // End sequence after desired number of frames
      if (frameCount >= ANIMATION_CONFIG.sequenceLength) {
        clearInterval(sequenceInterval);
        animatingRef.current = false;

        // Schedule the next animation sequence after a random pause
        setTimeout(() => {
          animateCharacter(setFrame, maxFrames, animatingRef);
        }, getRandomPause());
      }
    }, ANIMATION_CONFIG.frameSpeed);
  };

  // Animation references
  const tAnimating = React.useRef(false);
  const oAnimating = React.useRef(false);
  const nAnimating = React.useRef(false);
  const kAnimating = React.useRef(false);

  // Start animations with delays
  useEffect(() => {
    const tDelay = Math.random() * 1000;
    const oDelay = Math.random() * 2000 + 300;
    const nDelay = Math.random() * 1000 + 600;
    const kDelay = Math.random() * 500 + 900;

    // Initial animation sequences
    const tTimeout = setTimeout(
      () => animateCharacter(setTFrame, T_FRAMES.length, tAnimating),
      tDelay,
    );

    const oTimeout = setTimeout(
      () => animateCharacter(setOFrame, O_FRAMES.length, oAnimating),
      oDelay,
    );

    const nTimeout = setTimeout(
      () => animateCharacter(setNFrame, N_FRAMES.length, nAnimating),
      nDelay,
    );

    const kTimeout = setTimeout(
      () => animateCharacter(setKFrame, K_FRAMES.length, kAnimating),
      kDelay,
    );

    // Cleanup timeouts on unmount
    return () => {
      clearTimeout(tTimeout);
      clearTimeout(oTimeout);
      clearTimeout(nTimeout);
      clearTimeout(kTimeout);
    };
  }, []);

  // Get rotated star border
  const getStarBorder = () => {
    return STAR_CHARS.slice(starIndex) + STAR_CHARS.slice(0, starIndex);
  };

  return (
    <Box flexDirection="column" alignItems="flex-start">
      {/* Star border top */}
      <Text>{getStarBorder()}</Text>

      {/* Main ASCII art - each letter in its own column */}
      <Box marginY={1}>
        <Box flexDirection="row">
          {/* T character */}
          <Box marginRight={1}>
            <Text color="green">{T_FRAMES[tFrame]}</Text>
          </Box>

          {/* O character */}
          <Box marginRight={1}>
            <Text color="magenta">{O_FRAMES[oFrame]}</Text>
          </Box>

          {/* N character */}
          <Box marginRight={1}>
            <Text color="cyan">{N_FRAMES[nFrame]}</Text>
          </Box>

          {/* K character */}
          <Box>
            <Text color="yellow">{K_FRAMES[kFrame]}</Text>
          </Box>
        </Box>
      </Box>

      {/* Star border bottom */}
      <Text>{getStarBorder()}</Text>

      {/* Welcome message */}
      <WelcomeMessage />
    </Box>
  );
};

// Function to render the animation and return a function to stop it
const displayTonkAnimation = () => {
  // Render the component using Ink
  const {unmount} = render(<TonkAsciiAnimated />);

  // Return a cleanup function
  return () => {
    unmount();
  };
};

export default displayTonkAnimation;
