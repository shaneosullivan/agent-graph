import examples from "@/lib/examples.json";

/**
 * The home page's examples: each opens in a new tab, in the viewer, all
 * set up (app/examples/[slug]).
 */
export function Examples() {
  return (
    <section className="section" id="examples">
      <h2>See it in action</h2>
      <p className="section-lead">
        Example sessions, each showing something the viewer does. Each opens in
        a new tab: step through it with the timeline, and try the graph view.
      </p>
      <ul className="examples">
        {examples.map(example => (
          <li key={example.slug}>
            <a
              className="example"
              href={example.href}
              target="_blank"
              rel="noopener">
              <span className="example-title">{example.title}</span>
              <span className="example-description">{example.description}</span>
              <span className="example-tags">
                {example.tags.map(tag => (
                  <span key={tag} className="example-tag">
                    {tag}
                  </span>
                ))}
              </span>
            </a>
          </li>
        ))}
      </ul>
    </section>
  );
}
