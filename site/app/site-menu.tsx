"use client";

import {link} from "@/lib/links";
import {useEffect, useRef, useState, useSyncExternalStore} from "react";

/**
 * On a narrow screen, the header's links (all but "Log in" or "Account")
 * are behind a menu button, in a panel that slides in from the right
 * (site.css hides the header's own copies there). Escape, the backdrop, a
 * close button or a link closes it. Nothing's shown with nothing to list.
 */
export function SiteMenu({
  links,
}: {
  links: Array<{href: string; label: string}>;
}) {
  const admin = useSyncExternalStore(
    () => () => {},
    () => document.cookie.split("; ").includes("ag_admin=1"),
    () => false,
  );
  const [open, setOpen] = useState(false);
  const opener = useRef<HTMLButtonElement>(null);
  const closer = useRef<HTMLButtonElement>(null);
  const wasOpen = useRef(false);

  useEffect(() => {
    if (!open) {
      // Back to the button that opened it, once it's closed.
      if (wasOpen.current) {
        opener.current?.focus();
      }
      wasOpen.current = false;
      return;
    }
    wasOpen.current = true;
    closer.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
      }
    };
    // The page behind it stays put while it's open.
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    document.addEventListener("keydown", onKey);
    return () => {
      document.body.style.overflow = overflow;
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const items = admin ? [...links, {href: "/admin", label: "Admin"}] : links;
  if (!items.length) {
    return null;
  }
  const close = () => setOpen(false);
  return (
    <>
      <button
        ref={opener}
        type="button"
        className="menu-button"
        aria-label="Menu"
        aria-expanded={open}
        aria-controls="site-menu"
        onClick={() => setOpen(true)}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M4 7h16M4 12h16M4 17h16" />
        </svg>
      </button>
      <div
        className={`menu-backdrop${open ? " open" : ""}`}
        aria-hidden="true"
        onClick={close}
      />
      <div
        id="site-menu"
        className={`menu-panel${open ? " open" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label="Menu"
        inert={!open}>
        <button
          ref={closer}
          type="button"
          className="menu-close"
          aria-label="Close menu"
          onClick={close}>
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
        <ul>
          {items.map(item => (
            <li key={item.href}>
              <a {...link(item.href)} onClick={close}>
                {item.label}
              </a>
            </li>
          ))}
        </ul>
      </div>
    </>
  );
}
