import React from "react";
import styles from "./Link.module.css";

interface LinkProps {
  children: string | React.ReactNode;
  href?: string;
  target?: "_blank" | "_self" | "_parent" | "_top";
  rel?: string;
  className?: string;
  linkType: LinkType;
  onClick?: (event: React.MouseEvent<HTMLAnchorElement>) => void;
}

export enum LinkType {
  "External",
  "Internal",
  "Action",
}

const getLinkTypeClass = (linkType: LinkType): string => {
  switch (linkType) {
    case LinkType.External:
      return styles.external;
    case LinkType.Internal:
      return styles.internal;
    case LinkType.Action:
      return styles.action;
    default:
      return styles.external;
  }
};

export const Link: React.FC<LinkProps> = ({
  children,
  href = "",
  target,
  rel,
  linkType,
  className,
  onClick,
}) => (
  <a
    href={href}
    target={target}
    rel={target === "_blank" ? "noopener noreferrer" : rel}
    className={`${styles.link} ${getLinkTypeClass(linkType)} ${className || ""}`}
    onClick={onClick}
  >
    {children}
  </a>
);

export default Link;
