import {SiteFooter} from "./site-footer";
import {SiteHeader} from "./site-header";

/**
 * A page of text about the site (About, Contact, Privacy, Terms): the
 * header, a heading, the text, and the footer.
 */
export function InfoPage({
  title,
  updated,
  children,
}: {
  title: string;
  /** When it last changed, for the legal pages: "30 September 2026". */
  updated?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="site">
      <div className="page">
        <SiteHeader
          links={[
            {href: "/", label: "Home"},
            {href: "/docs", label: "Docs"},
          ]}
        />
        <article className="info prose">
          <h1>{title}</h1>
          {updated ? <p className="muted">Last updated {updated}</p> : null}
          {children}
        </article>
        <SiteFooter />
      </div>
    </div>
  );
}
