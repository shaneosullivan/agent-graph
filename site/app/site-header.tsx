import {CONTACT} from "@/lib/contact";
import {link} from "@/lib/links";

import {AccountLink} from "./account-link";
import {Brand} from "./brand";
import {SiteMenu} from "./site-menu";

/**
 * The site's header: the brand, the page's own links, the source on
 * GitHub, and Log in or Account. On a narrow screen the page's links (and
 * GitHub, and Admin) are in a menu instead (SiteMenu).
 */
export function SiteHeader({
  links = [],
}: {
  links?: Array<{href: string; label: string}>;
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
        <SiteMenu links={all} />
      </nav>
    </header>
  );
}
