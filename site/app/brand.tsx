import Link from "next/link";

export function Brand() {
  return (
    <Link href="/" className="brand">
      {/* The logo, lighter in a dark theme: app/icon.svg picks itself. */}
      <img src="/icon.svg" alt="" width={28} height={28} />
      <span>Agent Graph</span>
    </Link>
  );
}
