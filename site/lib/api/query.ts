// Reading a request's query (site/openapi.json): each endpoint says what it
// takes, and anything else is refused (`parameter_unknown`), as is a value
// of the wrong kind (`parameter_invalid`). Arrays and ranges are written
// with brackets, as Stripe's are: `expand[]=parent&expand[]=root`,
// `state[]=working`, `created[gte]=1760000000000`.

import {parameterInvalid, parameterUnknown} from "./errors";
import {parseEventId} from "./ids";

/** A range of times, in Unix milliseconds. */
export type Range = {gt?: number; gte?: number; lt?: number; lte?: number};

/** A point in a graph's history: an event, or a time (`as_of`). */
export type AsOf = {event: string} | {time: number};

export type Param =
  | {kind: "string"; max?: number}
  | {kind: "int"; min?: number; max?: number}
  | {kind: "bool"}
  | {kind: "enum"; values: ReadonlyArray<string>}
  | {kind: "strings"; max?: number}
  | {kind: "enums"; values: ReadonlyArray<string>}
  | {kind: "range"}
  | {kind: "asOf"};

type ValueOf<P extends Param> = P extends {kind: "string"}
  ? string
  : P extends {kind: "int"}
    ? number
    : P extends {kind: "bool"}
      ? boolean
      : P extends {kind: "enum"}
        ? string
        : P extends {kind: "strings"} | {kind: "enums"}
          ? Array<string>
          : P extends {kind: "range"}
            ? Range
            : AsOf;

export type Parsed<S extends Record<string, Param>> = {
  [K in keyof S]?: ValueOf<S[K]>;
};

const RANGE_KEYS = ["gt", "gte", "lt", "lte"] as const;

/**
 * The query of `url`, read as `spec` says: each parameter's value, of its
 * kind, or an `ApiError` for one it doesn't take or can't read.
 */
export function parseQuery<S extends Record<string, Param>>(
  url: URL,
  spec: S,
): Parsed<S> {
  const out: Record<string, unknown> = {};
  const seen = new Set<string>();
  for (const [key, value] of url.searchParams) {
    // name, name[], name[3], name[gte]
    const m = /^([a-z_]+)(?:\[([a-z]*|\d+)\])?$/.exec(key);
    const name = m?.[1] ?? key;
    const sub = m?.[2];
    const param = m ? spec[name] : undefined;
    if (!param) {
      throw parameterUnknown(name);
    }
    if (param.kind === "strings" || param.kind === "enums") {
      if (sub !== undefined && sub !== "" && !/^\d+$/.test(sub)) {
        throw parameterUnknown(key);
      }
      if (param.kind === "enums" && !param.values.includes(value)) {
        throw parameterInvalid(
          name,
          `Invalid ${name}: ${JSON.stringify(value)}. It's one of ${param.values.join(", ")}.`,
        );
      }
      const list = (out[name] ??= []) as Array<string>;
      list.push(value);
      if (param.kind === "strings" && param.max && list.length > param.max) {
        throw parameterInvalid(
          name,
          `Too many ${name}: at most ${param.max} are allowed.`,
        );
      }
      continue;
    }
    if (param.kind === "range") {
      if (!sub || !(RANGE_KEYS as ReadonlyArray<string>).includes(sub)) {
        throw parameterInvalid(
          name,
          `Invalid ${name}: give it as ${name}[gt], ${name}[gte], ${name}[lt] or ${name}[lte].`,
        );
      }
      const range = (out[name] ??= {}) as Range;
      range[sub as (typeof RANGE_KEYS)[number]] = integer(
        `${name}[${sub}]`,
        value,
      );
      continue;
    }
    if (sub !== undefined) {
      throw parameterUnknown(key);
    }
    if (seen.has(name)) {
      throw parameterInvalid(name, `${name} was given more than once.`);
    }
    seen.add(name);
    out[name] = scalar(name, value, param);
  }
  return out as Parsed<S>;
}

function scalar(name: string, value: string, param: Param): unknown {
  switch (param.kind) {
    case "string":
      if (!value || (param.max && value.length > param.max)) {
        throw parameterInvalid(
          name,
          value
            ? `Invalid ${name}: at most ${param.max} characters.`
            : `Invalid ${name}: it's empty.`,
        );
      }
      return value;
    case "int": {
      const n = integer(name, value);
      if (
        (param.min !== undefined && n < param.min) ||
        (param.max !== undefined && n > param.max)
      ) {
        throw parameterInvalid(
          name,
          param.max !== undefined
            ? `Invalid ${name}: must be between ${param.min ?? 0} and ${param.max}.`
            : `Invalid ${name}: must be at least ${param.min}.`,
        );
      }
      return n;
    }
    case "bool":
      if (value !== "true" && value !== "false") {
        throw parameterInvalid(name, `Invalid ${name}: must be true or false.`);
      }
      return value === "true";
    case "enum":
      if (!param.values.includes(value)) {
        throw parameterInvalid(
          name,
          `Invalid ${name}: ${JSON.stringify(value)}. It's one of ${param.values.join(", ")}.`,
        );
      }
      return value;
    case "asOf":
      return asOf(name, value);
    default:
      throw parameterUnknown(name);
  }
}

function integer(name: string, value: string): number {
  if (!/^-?\d{1,16}$/.test(value)) {
    throw parameterInvalid(name, `Invalid ${name}: must be an integer.`);
  }
  return Number(value);
}

/** `as_of`: an event id, or a Unix time in milliseconds. */
function asOf(name: string, value: string): AsOf {
  if (/^\d{1,16}$/.test(value)) {
    return {time: Number(value)};
  }
  const event = parseEventId(value);
  if (!event) {
    throw parameterInvalid(
      name,
      `Invalid ${name}: give an event id (evt_…) or a Unix time in milliseconds.`,
    );
  }
  return {event};
}

/** Whether `ms` is in `range`. */
export function inRange(ms: number | null, range: Range | undefined): boolean {
  if (!range) {
    return true;
  }
  if (ms === null) {
    return false;
  }
  return (
    (range.gt === undefined || ms > range.gt) &&
    (range.gte === undefined || ms >= range.gte) &&
    (range.lt === undefined || ms < range.lt) &&
    (range.lte === undefined || ms <= range.lte)
  );
}

/** The parameters every list takes. */
export const LIST_PARAMS = {
  limit: {kind: "int", min: 1, max: 1000},
  starting_after: {kind: "string", max: 200},
  ending_before: {kind: "string", max: 200},
} as const satisfies Record<string, Param>;

/** How many a list returns unless asked for more or fewer. */
export const DEFAULT_LIMIT = 100;
