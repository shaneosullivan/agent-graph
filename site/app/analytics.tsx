"use client";

import {usePathname} from "next/navigation";
import {useEffect} from "react";

import {enableAnalytics, trackPageview} from "@/lib/analytics-client";

/**
 * Reports each page shown (lib/analytics-client.ts): rendered by the
 * layout only when counting's on (ANALYTICS_DISABLED isn't true).
 */
export function Analytics() {
  const path = usePathname();
  useEffect(() => {
    enableAnalytics();
    trackPageview();
  }, [path]);
  return null;
}
