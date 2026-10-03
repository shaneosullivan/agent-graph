"use client";

import {useEffect, useRef, useState} from "react";

/**
 * A YouTube video, embedded at the width it's given, 16:9, from YouTube's
 * privacy-enhanced domain (no cookies until someone plays it themselves).
 * `id` is the video's id: the part after youtu.be/, or after watch?v=.
 *
 * With `autoplay`, it plays by itself once it's (half) on screen: at once,
 * if it is when the page loads, or when it's scrolled to. Until then it's
 * an empty frame, and nothing's fetched from YouTube. It plays muted
 * (browsers only let a muted video play by itself), and inline on a phone;
 * the viewer can turn the sound on. Being silent, it starts with its
 * subtitles on (in English); the viewer can turn them off.
 */
export function YouTubeVideo({
  id,
  title,
  autoplay = false,
  loop = false,
}: {
  id: string;
  /** What it's of, for screen readers (the frame's title). */
  title: string;
  autoplay?: boolean;
  loop?: boolean;
}) {
  const frame = useRef<HTMLDivElement>(null);
  const [inView, setInView] = useState(false);

  useEffect(() => {
    const el = frame.current;
    if (!autoplay || !el) {
      return;
    }
    const seen = new IntersectionObserver(
      entries => {
        if (entries.some(e => e.isIntersecting)) {
          setInView(true);
          seen.disconnect();
        }
      },
      {threshold: 0.5},
    );
    seen.observe(el);
    return () => seen.disconnect();
  }, [autoplay]);

  const params = new URLSearchParams({
    // Its related videos only from the same channel, and a plain player.
    rel: "0",
    modestbranding: "1",
    playsinline: "1",
    // High definition, where it can: YouTube no longer promises to honour
    // this (it picks the quality from the player's size and the connection),
    // but it's still a hint some browsers take.
    vq: "hd1080",
    ...(autoplay
      ? {autoplay: "1", mute: "1", cc_load_policy: "1", cc_lang_pref: "en"}
      : {}),
    // (Looping one video needs it named as its own playlist.)
    ...(loop ? {loop: "1", playlist: id} : {}),
  });
  const shown = !autoplay || inView;
  return (
    <div className="video" ref={frame}>
      {shown ? (
        <iframe
          src={`https://www.youtube-nocookie.com/embed/${encodeURIComponent(id)}?${params}`}
          title={title}
          allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
          allowFullScreen
          referrerPolicy="strict-origin-when-cross-origin"
        />
      ) : null}
    </div>
  );
}
