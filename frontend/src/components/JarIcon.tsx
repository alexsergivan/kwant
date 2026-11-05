import React from 'react';

/** The Idea jar's line icon (spec §2.1): lucide-style, 24 units, 2 px round stroke. Not lucide's Amphora. */
export const JarIcon: React.FC<{ size?: number; className?: string }> = ({ size = 15, className = '' }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={2}
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
    focusable="false"
    className={className}
    data-jar-icon
  >
    <path d="M8 3h8M7 6h10" />
    <path d="M8 6v1.5L5.5 10V19a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-9L16 7.5V6" />
    <path d="M9 15h2M13 17h2M10 18.5h1" />
  </svg>
);
