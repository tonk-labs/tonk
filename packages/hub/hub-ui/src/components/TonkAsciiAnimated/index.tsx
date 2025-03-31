import React, { useState, useEffect, useRef } from "react";
import styles from "./TonkAsciiAnimated.module.css";

interface TonkAsciiAnimatedProps {}

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

// Star border characters
const STAR_CHARS =
  "・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜☆。・゜☆。・゜";

// Animation configuration
const ANIMATION_CONFIG = {
  // Time between frame changes during an animation sequence (milliseconds)
  frameSpeed: 500,
  // Number of frames to show in a sequence
  sequenceLength: 3,
  // Min and max time between animation sequences (milliseconds)
  minPauseBetweenSequences: 1000,
  maxPauseBetweenSequences: 4000,
};

const TonkAsciiAnimated: React.FC<TonkAsciiAnimatedProps> = () => {
  // States to track current frame for each letter
  const [tFrame, setTFrame] = useState(0);
  const [oFrame, setOFrame] = useState(0);
  const [nFrame, setNFrame] = useState(0);
  const [kFrame, setKFrame] = useState(0);

  // Animated star border state with individual stars
  const [starBorder, setStarBorder] = useState<React.ReactNode[]>([]);

  // Refs to track if a character is currently animating
  const tAnimating = useRef(false);
  const oAnimating = useRef(false);
  const nAnimating = useRef(false);
  const kAnimating = useRef(false);

  // Helper to get random pause duration
  const getRandomPause = () => {
    return Math.floor(
      ANIMATION_CONFIG.minPauseBetweenSequences +
        Math.random() *
          (ANIMATION_CONFIG.maxPauseBetweenSequences -
            ANIMATION_CONFIG.minPauseBetweenSequences)
    );
  };

  // Initialize star border with individual spans
  useEffect(() => {
    const starElements = STAR_CHARS.split("").map((char, i) => (
      <span
        key={i}
        className={styles.starChar}
        style={{
          animationDelay: `${Math.random() * 5}s`,
          animationDuration: `${2 + Math.random() * 3}s`,
        }}
      >
        {char}
      </span>
    ));

    setStarBorder(starElements);
  }, []);

  // Animation function for a letter that runs a quick sequence then pauses
  const animateCharacter = (
    setFrame: React.Dispatch<React.SetStateAction<number>>,
    maxFrames: number,
    animatingRef: React.MutableRefObject<boolean>
  ) => {
    // Don't start a new animation if one is already running
    if (animatingRef.current) return;

    animatingRef.current = true;
    let frameCount = 0;

    // Start quick animation sequence
    const sequenceInterval = setInterval(() => {
      setFrame((prev) => (prev + 1) % maxFrames);
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

  // Start animation sequences with initial random delays
  useEffect(() => {
    // Start each character animation with a different initial delay
    const tDelay = Math.random() * 1000;
    const oDelay = Math.random() * 2000 + 300;
    const nDelay = Math.random() * 1000 + 600;
    const kDelay = Math.random() * 500 + 900;

    // Initial animation sequences
    setTimeout(
      () => animateCharacter(setTFrame, T_FRAMES.length, tAnimating),
      tDelay
    );
    setTimeout(
      () => animateCharacter(setOFrame, O_FRAMES.length, oAnimating),
      oDelay
    );
    setTimeout(
      () => animateCharacter(setNFrame, N_FRAMES.length, nAnimating),
      nDelay
    );
    setTimeout(
      () => animateCharacter(setKFrame, K_FRAMES.length, kAnimating),
      kDelay
    );

    // No need for cleanup because the sequences manage themselves
  }, []);

  return (
    <div className={styles.container}>
      {/* Star border top - individualized stars for animation */}
      <pre className={styles.starBorder}>{starBorder}</pre>

      {/* Main ASCII art layout */}
      <div className={styles.asciiOuterContainer}>
        {/* Each character in its own container for independent animation */}
        <div className={styles.characterRow}>
          {/* T character in its own container */}
          <div className={`${styles.characterWrapper} ${styles.letterT}`}>
            <pre className={styles.characterArt}>{T_FRAMES[tFrame]}</pre>
          </div>

          {/* O character in its own container */}
          <div className={`${styles.characterWrapper} ${styles.letterO}`}>
            <pre className={styles.characterArt}>{O_FRAMES[oFrame]}</pre>
          </div>

          {/* N character in its own container */}
          <div className={`${styles.characterWrapper} ${styles.letterN}`}>
            <pre className={styles.characterArt}>{N_FRAMES[nFrame]}</pre>
          </div>

          {/* K character in its own container */}
          <div className={`${styles.characterWrapper} ${styles.letterK}`}>
            <pre className={styles.characterArt}>{K_FRAMES[kFrame]}</pre>
          </div>
        </div>
      </div>

      {/* Star border bottom - same animation as top */}
      <pre className={styles.starBorder}>{starBorder}</pre>
    </div>
  );
};

export default TonkAsciiAnimated;
