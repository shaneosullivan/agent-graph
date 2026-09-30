/** The site's own address, whose links stay in the same tab. */
const SITE = "https://agentgraph.chofter.com";

/**
 * A link to `href`, as an `<a>`'s attributes: `<a {...link(url)}>`. One to
 * another site opens in a new tab (without that site getting a hold of
 * this one); one to this site doesn't.
 */
export function link(href: string): {
  href: string;
  target?: "_blank";
  rel?: string;
} {
  const offSite =
    /^https?:\/\//i.test(href) && href !== SITE && !href.startsWith(`${SITE}/`);
  return offSite
    ? {href, target: "_blank", rel: "noopener noreferrer"}
    : {href};
}
