/** The workstation's one icon set: 20-unit grid, 1.75 stroke, round joins.
 * Course pictograms (start triangle, control circle, finish double circle)
 * are drawn here too so the panel and the map share one vocabulary. */
const PATHS = {
  chevronRight: <path d="M8 5l5 5-5 5" />,
  chevronDown: <path d="M5 8l5 5 5-5" />,
  close: <path d="M5 5l10 10M15 5L5 15" />,
  prev: <path d="M6 5v10M15 5l-6 5 6 5z" />,
  next: <path d="M14 5v10M5 5l6 5-6 5z" />,
  play: <path d="M6 4.5l10 5.5-10 5.5z" />,
  pause: <path d="M7 5v10M13 5v10" />,
  latest: <path d="M4 5l5 5-5 5M10 5l5 5-5 5M16.5 5v10" />,
  star: <path d="M10 3l2.1 4.6 5 .5-3.8 3.3 1.1 4.9L10 13.8 5.6 16.3l1.1-4.9L2.9 8.1l5-.5z" />,
  start: <path d="M10 3.5l7 12H3z" />,
  control: <circle cx="10" cy="10" r="6" />,
  finish: (
    <>
      <circle cx="10" cy="10" r="7" />
      <circle cx="10" cy="10" r="4" />
    </>
  ),
  forecast: <path d="M3 15l4-5 3 3 4-6 3 4M3 17.5h14" />,
  settings: (
    <>
      <path d="M3 6h14M3 14h14" />
      <circle cx="7" cy="6" r="2" fill="var(--g2)" />
      <circle cx="13" cy="14" r="2" fill="var(--g2)" />
    </>
  ),
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, filled = false, size = 18 }: { name: IconName; filled?: boolean; size?: number }) {
  return (
    <svg
      className="icon"
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {PATHS[name]}
    </svg>
  );
}
