import {AccountLink} from "./account-link";
import {Brand} from "./brand";
import {SiteMenu} from "./site-menu";

/**
 * The site's header: the brand, the page's own links, and Log in or
 * Account. On a narrow screen the page's links (and Admin) are in a menu
 * instead (SiteMenu).
 */
export function SiteHeader({
  links = [],
}: {
  links?: Array<{href: string; label: string}>;
}) {
  return (
    <header className="top">
      <Brand />
      <nav>
        {links.map(link => (
          <a key={link.href} className="nav-page" href={link.href}>
            {link.label}
          </a>
        ))}
        <AccountLink />
        <SiteMenu links={links} />
      </nav>
    </header>
  );
}
