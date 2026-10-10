// Each key's rate limit (site/openapi.json, "Rate limits"): 25 requests a
// second, in bursts of up to 100, as a token bucket. It's kept by each
// server instance, so it's a limit per instance: a key's requests spread
// over several can go faster, though never much (a busy key's requests
// tend to land on the instances already running).

export const RATE_PER_SECOND = 25;
export const BURST = 100;

/** How many buckets are kept before the quiet ones are dropped. */
const MOST_BUCKETS = 10_000;

export type Taken = {
  ok: boolean;
  limit: number;
  remaining: number;
  /** Seconds until the bucket's full again. */
  reset: number;
  /** When refused: seconds until a request would be let through. */
  retryAfter: number;
};

export class RateLimiter {
  private buckets = new Map<string, {tokens: number; at: number}>();
  private readonly rate: number;
  private readonly burst: number;
  private readonly now: () => number;

  constructor(
    rate = RATE_PER_SECOND,
    burst = BURST,
    now: () => number = Date.now,
  ) {
    this.rate = rate;
    this.burst = burst;
    this.now = now;
  }

  private bucket(key: string) {
    const now = this.now();
    let b = this.buckets.get(key);
    if (!b) {
      if (this.buckets.size >= MOST_BUCKETS) {
        this.prune(now);
      }
      b = {tokens: this.burst, at: now};
      this.buckets.set(key, b);
    }
    b.tokens = Math.min(
      this.burst,
      b.tokens + ((now - b.at) / 1000) * this.rate,
    );
    b.at = now;
    return b;
  }

  /** Takes one request from `key`'s bucket, if there's one to take. */
  take(key: string): Taken {
    const b = this.bucket(key);
    const ok = b.tokens >= 1;
    if (ok) {
      b.tokens -= 1;
    }
    return {
      ok,
      limit: this.rate,
      remaining: Math.floor(b.tokens),
      reset: Math.ceil((this.burst - b.tokens) / this.rate),
      retryAfter: ok ? 0 : Math.max(1, Math.ceil((1 - b.tokens) / this.rate)),
    };
  }

  /** Gives back a request that didn't count (a `304`). */
  giveBack(key: string): void {
    const b = this.bucket(key);
    b.tokens = Math.min(this.burst, b.tokens + 1);
  }

  /** Drops buckets that have filled up again: they'd start full anyway. */
  private prune(now: number) {
    for (const [key, b] of this.buckets) {
      if (b.tokens + ((now - b.at) / 1000) * this.rate >= this.burst) {
        this.buckets.delete(key);
      }
    }
  }
}
