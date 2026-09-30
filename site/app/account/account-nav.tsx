"use client";

import {useEffect, useState} from "react";

import {
  AlertIcon,
  CardIcon,
  KeyIcon,
  LaptopIcon,
  LiveIcon,
  OverviewIcon,
} from "./icons";

const SECTIONS = [
  {id: "overview", label: "Overview", icon: <OverviewIcon />},
  {id: "billing", label: "Billing", icon: <CardIcon />},
  {id: "live-sharing", label: "Live sharing", icon: <LiveIcon />},
  {id: "computers", label: "Computers", icon: <LaptopIcon />},
  {id: "api-tokens", label: "API tokens", icon: <KeyIcon />},
  {id: "delete", label: "Delete account", icon: <AlertIcon />, danger: true},
];

/**
 * The account page's navigation: a link to each of its panels, the one
 * being read marked as it's scrolled to.
 */
export function AccountNav() {
  const [current, setCurrent] = useState("overview");
  useEffect(() => {
    // The panel nearest the top third of the window is the one being read.
    const seen = new IntersectionObserver(
      entries => {
        const shown = entries.filter(e => e.isIntersecting);
        if (shown.length) {
          setCurrent(shown[0].target.id);
        }
      },
      {rootMargin: "-25% 0px -65% 0px"},
    );
    for (const {id} of SECTIONS) {
      const el = document.getElementById(id);
      if (el) {
        seen.observe(el);
      }
    }
    return () => seen.disconnect();
  }, []);
  return (
    <nav className="acct-nav" aria-label="Account">
      {SECTIONS.map(s => (
        <a
          key={s.id}
          href={`#${s.id}`}
          className={s.danger ? "acct-nav-danger" : undefined}
          aria-current={current === s.id ? "true" : undefined}>
          {s.icon} {s.label}
        </a>
      ))}
    </nav>
  );
}
