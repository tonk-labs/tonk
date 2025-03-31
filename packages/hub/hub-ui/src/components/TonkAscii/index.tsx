import React from "react";
import styles from "./TonkAscii.module.css";

interface TonkAsciiProps {}

const ASCII = `
・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜

       _____   U  ___ u  _   _       _  __    
      |_ " _|   \\/"_ \\/ | \\ |"|     |"|/ /    
        | |     | | | |<|  \\| |>    | ' /     
       /| |\\.-,_| |_| |U| |\\  |u  U/| . \\\\u   
      u |_|U \\_)-\\___/  |_| \\_|     |_|\\_\\    
      _// \\\\_     \\\\    ||   \\\\,-.,-,>> \\\\,-. 
     (__) (__)   (__)   (_")  (_/  \\.)   (_/  

・。゜☆。・゜。・。゜☆。・゜★・。゜☆。・゜。・。゜☆。・゜☆。・゜
`;

const TonkAscii: React.FC<TonkAsciiProps> = () => {
  return (
    <div className={styles.container}>
      <pre className={styles.text}>{ASCII}</pre>
    </div>
  );
};

export default TonkAscii;
