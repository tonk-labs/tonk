import React, { forwardRef } from "react";
import styles from "./Button.module.css";

type ButtonColor = "blue" | "purple" | "green" | "ghost";
type ButtonVariant = "outline" | "filled";
type ButtonSize = "sm" | "md" | "lg";
type ButtonShape = "square" | "rounded" | "pill" | "default";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  color?: ButtonColor;
  variant?: ButtonVariant;
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

const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      color = "default",
      variant = "outline",
      size = "lg",
      shape = "default",
      children,
      className,
      tooltip,
      tooltipPosition = "top",
      disabled = false,
      ...props
    },
    ref
  ) => {
    const buttonClasses = [
      styles.button,
      variant === "outline" && styles.outline,
      styles[color],
      styles[toCamelCase("size", size)],
      styles[toCamelCase("shape", shape)],
      tooltip && styles.hasTooltip,
      disabled && styles.disabled,
      className,
    ]
      .filter(Boolean)
      .join(" ");

    return (
      <button
        ref={ref}
        className={buttonClasses}
        data-tooltip={tooltip}
        data-tooltip-position={tooltip ? tooltipPosition : undefined}
        {...props}
      >
        {children}
      </button>
    );
  }
);

// Add display name for debugging
Button.displayName = "Button";

export default Button;
