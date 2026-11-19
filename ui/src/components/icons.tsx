// small inline icons so phase 0 does not pull in an icon library yet
const base = {
  width: 14,
  height: 14,
  viewBox: "0 0 16 16",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

export const PauseIcon = () => (
  <svg {...base} aria-hidden>
    <path d="M5.5 3.5v9M10.5 3.5v9" />
  </svg>
);

export const PlayIcon = () => (
  <svg {...base} aria-hidden>
    <path d="M5 3.5l7 4.5-7 4.5z" />
  </svg>
);

export const CloseIcon = () => (
  <svg {...base} aria-hidden>
    <path d="M4 4l8 8M12 4l-8 8" />
  </svg>
);

export const RetryIcon = () => (
  <svg {...base} aria-hidden>
    <path d="M3 8a5 5 0 1 0 1.5-3.5M3 2.5v2.5h2.5" />
  </svg>
);
