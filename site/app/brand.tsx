import Link from "next/link";

export function Brand() {
  return (
    <Link href="/" className="brand">
      <svg viewBox="0 0 32 32" aria-hidden="true">
        <path
          d="M9 9v14M9 16h14"
          stroke="currentColor"
          strokeWidth="2.5"
          fill="none"
        />
        <circle cx="9" cy="8" r="4.5" className="b1" />
        <circle cx="24" cy="16" r="4.5" className="b2" />
        <circle cx="9" cy="24" r="4.5" className="b3" />
      </svg>
      <span>Agent Graph</span>
    </Link>
  );
}
