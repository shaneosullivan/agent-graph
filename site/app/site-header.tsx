import type {ReactNode} from "react";

import {CONTACT} from "@/lib/contact";
import {link} from "@/lib/links";

import {AccountLink} from "./account-link";
import {Brand} from "./brand";
import {SiteMenu} from "./site-menu";

/** A link in the header, and any under it (in a dropdown, or indented in the menu). */
export type NavLink = {
  href: string;
  label: string;
  under?: Array<{href: string; label: string}>;
};

/** What's under "Docs" wherever it's a link: the graph API's own pages. */
const UNDER_DOCS = [
  {href: "/docs/reference", label: "API reference"},
  {href: "/showcase", label: "API showcase"},
];

/**
 * The site's header: the brand, the page's own links, the source on
 * GitHub, and Log in or Account. On a narrow screen the page's links (and
 * GitHub, and Admin) are in a menu instead (SiteMenu), with `menu` below
 * them: more of the page's own, like the docs' contents. "Docs" has the
 * API reference and the API showcase under it.
 */
export function SiteHeader({
  links = [],
  menu,
}: {
  links?: Array<{href: string; label: string}>;
  /** More for the narrow screen's menu, under the links. */
  menu?: ReactNode;
}) {
  const all: Array<NavLink> = [
    ...links.map(l => (l.href === "/docs" ? {...l, under: UNDER_DOCS} : l)),
    {href: CONTACT.source, label: "GitHub"},
  ];
  return (
    <header className="top">
      <Brand />
      <nav>
        {all.map(item =>
          item.under ? (
            // A dropdown, on hover or focus.
            <span key={item.href} className="nav-dropdown">
              <a className="nav-page" {...link(item.href)}>
                {item.label}
              </a>
              <span className="nav-dropdown-list">
                {item.under.map(u => (
                  <a key={u.href} {...link(u.href)}>
                    {u.label}
                  </a>
                ))}
              </span>
            </span>
          ) : (
            <a key={item.href} className="nav-page" {...link(item.href)}>
              {item.label}
            </a>
          ),
        )}
        <AccountLink />
        <SiteMenu links={all}>{menu}</SiteMenu>
      </nav>
    </header>
  );
}
