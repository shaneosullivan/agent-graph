"use client";

import {useEffect, useState} from "react";

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
