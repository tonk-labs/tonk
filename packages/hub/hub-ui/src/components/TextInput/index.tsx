import React from "react";
import styles from "./TextInput.module.css";

interface TextInputProps extends React.InputHTMLAttributes<HTMLInputElement> {
    label?: string;
    error?: string;
    tooltip?: string;
    tooltipPosition?: "top" | "bottom" | "left" | "right";
}

const TextInput: React.FC<TextInputProps> = ({
    label,
    error,
    className,
    tooltip,
    tooltipPosition = "top",
    ...props
}) => {
    const inputClasses = [
        styles.input,
        error && styles.error,
        tooltip && styles.hasTooltip,
        className,
    ]
        .filter(Boolean)
        .join(" ");

    return (
        <div className={styles.container}>
            {label && <label className={styles.label}>{label}</label>}
            <input
                className={inputClasses}
                data-tooltip={tooltip}
                data-tooltip-position={tooltip ? tooltipPosition : undefined}
                {...props}
            />
            {error && <span className={styles.errorMessage}>{error}</span>}
        </div>
    );
};

export default TextInput;
