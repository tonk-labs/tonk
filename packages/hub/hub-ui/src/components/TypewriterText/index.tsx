import React, { useEffect, useState } from "react";
import { Text } from "..";

interface TypewriterTextProps {
    children: string;
    delay?: number;
    onComplete?: () => void;
    style?: React.CSSProperties;
}

const TypewriterText: React.FC<TypewriterTextProps> = ({
    children,
    delay = 50,
    onComplete,
    style,
}) => {
    const [displayText, setDisplayText] = useState("");
    const [currentIndex, setCurrentIndex] = useState(0);

    useEffect(() => {
        if (currentIndex < children.length) {
            const timeout = setTimeout(() => {
                setDisplayText((prev) => prev + children[currentIndex]);
                setCurrentIndex((prev) => prev + 1);
            }, delay);

            return () => clearTimeout(timeout);
        } else if (onComplete) {
            onComplete();
        }
    }, [currentIndex, children, delay, onComplete]);

    return <Text style={style}>{displayText}</Text>;
};

export default TypewriterText;
