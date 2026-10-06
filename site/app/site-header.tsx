import type {ReactNode} from "react";

import {CONTACT} from "@/lib/contact";
import {link} from "@/lib/links";

import {AccountLink} from "./account-link";
import {Brand} from "./brand";
import {SiteMenu} from "./site-menu";

/**
 * The site's header: the brand, the page's own links, the source on
 * GitHub, and Log in or Account. On a narrow screen the page's links (and
 * GitHub, and Admin) are in a menu instead (SiteMenu), with `menu` below
 * them: more of the page's own, like the docs' contents.
 */
export function SiteHeader({
  links = [],
  menu,
}: {
  links?: Array<{href: string; label: string}>;
  /** More for the narrow screen's menu, under the links. */
  menu?: ReactNode;
}) {
  const all = [...links, {href: CONTACT.source, label: "GitHub"}];
  return (
    <header className="top">
      <Brand />
      <nav>
        {all.map(item => (
          <a key={item.href} className="nav-page" {...link(item.href)}>
            {item.label}
          </a>
        ))}
        <AccountLink />
        <SiteMenu links={all}>{menu}</SiteMenu>
      </nav>
    </header>
  );
}
