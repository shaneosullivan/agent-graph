import { AccountLink } from "./account-link";
import { Brand } from "./brand";

/** The site's header: the brand, the page's own links, and Log in or Account. */
export function SiteHeader({ links = [] }: { links?: { href: string; label: string }[] }) {
  return (
    <header className="top">
      <Brand />
      <nav>
        {links.map((link) => (
          <a key={link.href} href={link.href}>
            {link.label}
          </a>
        ))}
        <AccountLink />
      </nav>
    </header>
  );
}
