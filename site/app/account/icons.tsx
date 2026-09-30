/**
 * The account page's icons: 24×24 line drawings, in the text's colour (so
 * they follow the theme), sized by their container.
 */

function Icon({children}: {children: React.ReactNode}) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      {children}
    </svg>
  );
}

/** Billing: a card. */
export function CardIcon() {
  return (
    <Icon>
      <rect x="2.5" y="5" width="19" height="14" rx="2.5" />
      <path d="M2.5 9.5h19M6.5 15h4" />
    </Icon>
  );
}

/** Sharing live: a signal, broadcasting. */
export function LiveIcon() {
  return (
    <Icon>
      <circle cx="12" cy="12" r="2" />
      <path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 7.8a6 6 0 0 1 0 8.4M5 5a10 10 0 0 0 0 14M19 5a10 10 0 0 1 0 14" />
    </Icon>
  );
}

/** A computer logged in. */
export function LaptopIcon() {
  return (
    <Icon>
      <rect x="4" y="5" width="16" height="11" rx="1.5" />
      <path d="M2 19.5h20" />
    </Icon>
  );
}

/** An API token: a key. */
export function KeyIcon() {
  return (
    <Icon>
      <circle cx="8" cy="15" r="4" />
      <path d="m10.8 12.2 8.7-8.7M16.5 6.5l2.5 2.5M14 9l2 2" />
    </Icon>
  );
}

/** Something that can't be undone. */
export function AlertIcon() {
  return (
    <Icon>
      <path d="M10.3 3.9 2.4 17.6A2 2 0 0 0 4.1 20.5h15.8a2 2 0 0 0 1.7-2.9L13.7 3.9a2 2 0 0 0-3.4 0Z" />
      <path d="M12 9.5v4M12 17h.01" />
    </Icon>
  );
}

/** At a glance: a grid of tiles. */
export function OverviewIcon() {
  return (
    <Icon>
      <rect x="3.5" y="3.5" width="7" height="7" rx="1.5" />
      <rect x="13.5" y="3.5" width="7" height="7" rx="1.5" />
      <rect x="3.5" y="13.5" width="7" height="7" rx="1.5" />
      <rect x="13.5" y="13.5" width="7" height="7" rx="1.5" />
    </Icon>
  );
}

/** Going somewhere: an arrow. */
export function ArrowIcon() {
  return (
    <Icon>
      <path d="M5 12h14M13 6l6 6-6 6" />
    </Icon>
  );
}
