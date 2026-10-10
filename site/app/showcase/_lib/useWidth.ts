"use client";

import {useEffect, useState, useSyncExternalStore} from "react";

/**
 * A ref for an element, and its width as it changes, so a chart can be
 * drawn at its real size (and its text stays its real size). A callback ref,
 * so it follows the element if a chart swaps one for another.
 */
export function useWidth<T extends HTMLElement = HTMLDivElement>(
  fallback = 800,
) {
  const [el, ref] = useState<T | null>(null);
  const [width, setWidth] = useState(fallback);
  useEffect(() => {
    if (!el) {
      return;
    }
    const seen = new ResizeObserver(([entry]) =>
      setWidth(Math.max(240, Math.floor(entry.contentRect.width))),
    );
    seen.observe(el);
    return () => seen.disconnect();
  }, [el]);
  return {ref, width};
}

/** The width below which the showcase is laid out for a phone (showcase.css). */
const PHONE = "(max-width: 780px)";

/** Whether the showcase is laid out for a phone now. */
export function usePhone(): boolean {
  return useSyncExternalStore(
    change => {
      const query = matchMedia(PHONE);
      query.addEventListener("change", change);
      return () => query.removeEventListener("change", change);
    },
    () => matchMedia(PHONE).matches,
    () => false,
  );
}
