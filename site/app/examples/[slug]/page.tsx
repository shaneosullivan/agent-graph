import type {Metadata} from "next";
import {notFound} from "next/navigation";

import examples from "@/lib/examples.json";

import {Viewer} from "../../viewer";

type Params = Promise<{slug: string}>;

/** Every example's page is built ahead of time. */
export function generateStaticParams() {
  return examples.map(({slug}) => ({slug}));
}

export async function generateMetadata({
  params,
}: {
  params: Params;
}): Promise<Metadata> {
  const {slug} = await params;
  const example = examples.find(e => e.slug === slug);
  return {
    title: example ? `${example.title} · Agent Graph example` : "Agent Graph",
    description: example?.description,
  };
}

/**
 * An example (lib/examples.json, made by scripts/build-examples.mjs), in
 * the viewer: its log is a file of its own (public/examples/<slug>.jsonl),
 * read whole, and shown as it stood at its last event.
 */
export default async function ExamplePage({params}: {params: Params}) {
  const {slug} = await params;
  if (!examples.some(e => e.slug === slug)) {
    notFound();
  }
  return (
    <Viewer
      id={`example-${slug}`}
      live={false}
      url={`/examples/${slug}.jsonl`}
    />
  );
}
