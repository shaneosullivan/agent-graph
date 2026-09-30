import {CONTACT} from "@/lib/contact";
import {link} from "@/lib/links";

// The X, Threads and GitHub logos (from Simple Icons), filled with the text colour.
const X_LOGO =
  "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z";
const GITHUB_LOGO =
  "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12";
const THREADS_LOGO =
  "M12.186 24h-.007c-3.581-.024-6.334-1.205-8.184-3.509C2.35 18.44 1.5 15.586 1.472 12.01v-.017c.03-3.579.879-6.43 2.525-8.482C5.845 1.205 8.6.024 12.18 0h.014c2.746.02 5.043.725 6.826 2.098 1.677 1.29 2.858 3.13 3.509 5.467l-2.04.569c-1.104-3.96-3.898-5.984-8.304-6.015-2.91.022-5.11.936-6.54 2.717C4.307 6.504 3.616 8.914 3.589 12c.027 3.086.718 5.496 2.057 7.164 1.43 1.783 3.631 2.698 6.54 2.717 2.623-.02 4.358-.631 5.8-2.045 1.647-1.613 1.618-3.593 1.09-4.798-.31-.71-.873-1.3-1.634-1.75-.192 1.352-.622 2.446-1.284 3.272-.886 1.102-2.14 1.704-3.73 1.79-1.202.065-2.361-.218-3.259-.801-1.063-.689-1.685-1.74-1.752-2.964-.065-1.19.408-2.285 1.33-3.082.88-.76 2.119-1.207 3.583-1.291a13.853 13.853 0 0 1 3.02.142c-.126-.742-.375-1.332-.75-1.757-.513-.586-1.308-.883-2.359-.89h-.029c-.844 0-1.992.232-2.721 1.32L7.734 7.847c.98-1.454 2.568-2.256 4.478-2.256h.044c3.194.02 5.097 1.975 5.287 5.388.108.046.216.094.321.142 1.49.7 2.58 1.761 3.154 3.07.797 1.82.871 4.79-1.548 7.158-1.85 1.81-4.094 2.628-7.277 2.65Zm1.003-11.69c-.242 0-.487.007-.739.021-1.836.103-2.98.946-2.916 2.143.067 1.256 1.452 1.839 2.784 1.767 1.224-.065 2.818-.543 3.086-3.71a10.5 10.5 0 0 0-2.215-.221z";

/**
 * The site's footer: its pages about itself (About, Contact, Privacy,
 * Terms), where else to find it, and `children` (a page's own note) above.
 */
export function SiteFooter({children}: {children?: React.ReactNode}) {
  return (
    <footer className="foot">
      {children ? <p className="foot-note">{children}</p> : null}
      <div className="foot-row">
        <nav className="foot-links" aria-label="About Agent Graph">
          <a href="/about">About</a>
          <a href="/contact">Contact</a>
          <a href="/privacy">Privacy</a>
          <a href="/terms">Terms &amp; Conditions</a>
        </nav>
        <div className="foot-social">
          <a
            className="foot-icon"
            {...link(CONTACT.twitter.url)}
            title={`${CONTACT.twitter.handle} on X`}
            aria-label={`${CONTACT.twitter.handle} on X`}>
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d={X_LOGO} />
            </svg>
          </a>
          <a
            className="foot-icon"
            {...link(CONTACT.threads.url)}
            title={`${CONTACT.threads.handle} on Threads`}
            aria-label={`${CONTACT.threads.handle} on Threads`}>
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d={THREADS_LOGO} />
            </svg>
          </a>
          <a
            className="foot-icon"
            {...link(CONTACT.source)}
            title="Agent Graph on GitHub"
            aria-label="Agent Graph on GitHub">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d={GITHUB_LOGO} />
            </svg>
          </a>
        </div>
      </div>
      <p className="foot-company">
        &copy; {new Date().getFullYear()} {CONTACT.company}, {CONTACT.country}
      </p>
    </footer>
  );
}
