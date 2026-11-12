import type { ReactNode } from "react";

const ICONS: Record<string, ReactNode> = {
  explorer: <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  tasks: <path d="M9 6h11M9 12h11M9 18h11M4 6l1 1 2-2M4 12l1 1 2-2M4 18l1 1 2-2" />,
  devices: <path d="M4 9a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3v6a2 2 0 0 1-2 2h-3l-2-3h-2l-2 3H6a2 2 0 0 1-2-2z" />,
  chevron: <path d="M6 9l6 6 6-6" />,
  plus: <path d="M12 5v14M5 12h14" />,
  mic: (
    <>
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
    </>
  ),
  send: <path d="M12 19V5M6 11l6-6 6 6" />,
  stop: <rect x="7" y="7" width="10" height="10" rx="1.5" fill="currentColor" />,
  "chevron-up": <path d="M6 15l6-6 6 6" />,
  history: (
    <>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 8v4l3 2" />
    </>
  ),
  compose: <path d="M12 20h8M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" />,
};

export function Icon({ name }: { name: keyof typeof ICONS }) {
  return (
    <svg
      className="icon"
      aria-hidden="true"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {ICONS[name]}
    </svg>
  );
}
