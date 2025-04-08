import React from "react";
import styles from "./Button.module.css";

type ButtonVariant = "blue" | "purple" | "green" | "ghost";
type ButtonSize = "sm" | "md" | "lg";
type ButtonShape = "square" | "rounded" | "pill";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
    variant: ButtonVariant;
    size?: ButtonSize;
    shape?: ButtonShape;
    children: React.ReactNode;
    tooltip?: string;
    tooltipPosition?: "top" | "bottom" | "left" | "right";
}

/**
 * Converts a string with kebab-case prefix to camelCase format for CSS modules
 * @param prefix The prefix to use (e.g., "size" or "shape")
 * @param value The value to append (e.g., "lg" or "rounded")
 * @returns The camelCased class name (e.g., "sizeLg" or "shapeRounded")
 */
const toCamelCase = (prefix: string, value: string): string => {
    return `${prefix}${value.charAt(0).toUpperCase() + value.slice(1)}`;
};

const Button: React.FC<ButtonProps> = ({
    variant,
    size = "lg",
    shape = "square",
    children,
    className,
    tooltip,
    tooltipPosition = "top",
    ...props
}) => {
    const buttonClasses = [
        styles.button,
        styles[variant],
        styles[toCamelCase("size", size)],
        styles[toCamelCase("shape", shape)],
        tooltip && styles.hasTooltip,
        className,
    ]
        .filter(Boolean)
        .join(" ");

    return (
        <button
            className={buttonClasses}
            data-tooltip={tooltip}
            data-tooltip-position={tooltip ? tooltipPosition : undefined}
            {...props}
        >
            {children}
        </button>
    );
};

export default Button;
