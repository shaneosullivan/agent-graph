"use client";

// Whose graphs the showcase reads: the demo account's, always to begin
// with; or, for a browser that's logged in, its own account's, if it
// switches (the toggle in the Shell). Each through a proxy of its own
// (lib/showcase.ts): the demo's with the site's key, cached for everyone;
// the account's by its session, for it alone.

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";

import {setNow} from "./format";

export type Source = "demo" | "mine";

/** Where each source's calls go. */
export const API_BASES: Record<Source, string> = {
  demo: "/showcase/api/ag",
  mine: "/showcase/api/mine",
};

type Sourcing = {
  source: Source;
  setSource: (source: Source) => void;
  /** Whether this browser's logged in (app/account-link.tsx's way). */
  signedIn: boolean;
};

const SourceContext = createContext<Sourcing>({
  source: "demo",
  setSource: () => {},
  signedIn: false,
});

export function SourceProvider({children}: {children: React.ReactNode}) {
  const [chosen, setChosen] = useState<Source>("demo");
  const signedIn = useSyncExternalStore(
    () => () => {},
    () => document.cookie.split("; ").includes("ag_signed_in=1"),
    () => false,
  );
  const source = signedIn ? chosen : "demo";
  const setSource = useCallback((next: Source) => {
    // "How long ago" is from the demo's last event, or from now.
    setNow(null);
    setChosen(next);
  }, []);
  const value = useMemo(
    () => ({source, setSource, signedIn}),
    [source, setSource, signedIn],
  );
  return (
    <SourceContext.Provider value={value}>{children}</SourceContext.Provider>
  );
}

export function useSource(): Sourcing {
  return useContext(SourceContext);
}
