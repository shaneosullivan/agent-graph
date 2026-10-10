// Searching nodes (site/openapi.json, "Search nodes"): a query is clauses,
// `field:"value"`, joined by AND or OR (not both in one query):
//
//   state:"working" AND stale:"true"
//   agent_type:"Explore" AND descendant_count>10
//   summary~"migration" AND -state:"completed"

import {searchQueryInvalid} from "./errors";
import type {ApiNode} from "./model";

type Op = ":" | "~" | ">" | ">=" | "<" | "<=";

export type Clause = {
  field: string;
  op: Op;
  /** A string (quoted), a number, or null. */
  value: string | number | null;
  not: boolean;
};

export type Query = {join: "AND" | "OR"; clauses: Array<Clause>};

type Kind = "text" | "number" | "bool" | "id";

/** What can be searched, and how: text and ids by value, numbers and times by size. */
const FIELDS: Record<string, {kind: Kind; of: (n: ApiNode) => unknown}> = {
  state: {kind: "text", of: n => n.state},
  kind: {kind: "text", of: n => n.kind},
  provider: {kind: "text", of: n => n.provider},
  agent_type: {kind: "text", of: n => n.agent_type},
  title: {kind: "text", of: n => n.title},
  summary: {kind: "text", of: n => n.summary},
  headline: {kind: "text", of: n => n.headline},
  purpose: {kind: "text", of: n => n.purpose},
  attention: {kind: "text", of: n => n.attention},
  stale: {kind: "bool", of: n => n.stale},
  blocked: {kind: "bool", of: n => n.blocked !== null},
  background: {kind: "bool", of: n => n.background},
  parent_id: {kind: "id", of: n => n.parent_id},
  root_id: {kind: "id", of: n => n.root_id},
  session_id: {kind: "id", of: n => n.session_id},
  depth: {kind: "number", of: n => n.depth},
  created: {kind: "number", of: n => n.created},
  ended: {kind: "number", of: n => n.ended},
  last_event: {kind: "number", of: n => n.last_event},
  child_count: {kind: "number", of: n => n.child_count},
  descendant_count: {kind: "number", of: n => n.descendant_count},
  open_tasks: {kind: "number", of: n => n.task_counts.open},
  blocked_open_tasks: {kind: "number", of: n => n.blocked?.open_tasks ?? null},
};

export const SEARCH_FIELDS = Object.keys(FIELDS);

/** Reads a query; `search_query_invalid`, saying where, if it can't be. */
export function parseSearch(text: string): Query {
  const tokens = tokenize(text);
  const clauses: Array<Clause> = [];
  let join: Query["join"] | null = null;
  let i = 0;
  const at = () => tokens[i];
  while (i < tokens.length) {
    if (clauses.length) {
      const word = at();
      if (word.kind !== "word" || (word.text !== "AND" && word.text !== "OR")) {
        throw searchQueryInvalid(
          `Expected AND or OR at character ${word.pos + 1}.`,
        );
      }
      if (join && join !== word.text) {
        throw searchQueryInvalid(
          "A query can join its clauses with AND or with OR, not both.",
        );
      }
      join = word.text;
      i += 1;
      if (i >= tokens.length) {
        throw searchQueryInvalid(`Expected a clause after ${word.text}.`);
      }
    }
    let not = false;
    if (at().kind === "minus") {
      not = true;
      i += 1;
    }
    const name = tokens[i];
    if (!name || name.kind !== "word") {
      throw searchQueryInvalid(
        `Expected a field name at character ${(name ?? tokens[i - 1]).pos + 1}.`,
      );
    }
    const field = FIELDS[name.text];
    if (!field) {
      throw searchQueryInvalid(
        `${name.text} can't be searched. These can: ${SEARCH_FIELDS.join(", ")}.`,
      );
    }
    const op = tokens[i + 1];
    if (!op || op.kind !== "op") {
      throw searchQueryInvalid(
        `Expected :, ~, >, >=, < or <= after ${name.text}.`,
      );
    }
    const value = tokens[i + 2];
    if (
      !value ||
      (value.kind !== "string" &&
        value.kind !== "number" &&
        value.kind !== "word")
    ) {
      throw searchQueryInvalid(
        `Expected a value after ${name.text}${op.text}.`,
      );
    }
    let v: Clause["value"];
    if (value.kind === "word") {
      if (value.text !== "null") {
        throw searchQueryInvalid(
          `Quote ${value.text}: ${name.text}${op.text}"${value.text}".`,
        );
      }
      v = null;
    } else {
      v = value.kind === "number" ? Number(value.text) : value.text;
    }
    const clause: Clause = {field: name.text, op: op.text as Op, value: v, not};
    check(clause, field.kind);
    clauses.push(clause);
    i += 3;
  }
  if (!clauses.length) {
    throw searchQueryInvalid("The query is empty.");
  }
  return {join: join ?? "AND", clauses};
}

/** Refuses what a field can't be compared with. */
function check(c: Clause, kind: Kind): void {
  if (c.value === null) {
    if (c.op !== ":") {
      throw searchQueryInvalid(
        `Use ${c.field}:null to find nodes without one.`,
      );
    }
    return;
  }
  if (c.op === "~") {
    if (kind !== "text") {
      throw searchQueryInvalid(
        `${c.field} can't be searched with ~: it isn't text.`,
      );
    }
    if (typeof c.value !== "string" || c.value.length < 3) {
      throw searchQueryInvalid(
        `${c.field}~ needs at least 3 characters to look for.`,
      );
    }
    return;
  }
  if (c.op !== ":") {
    if (kind !== "number" || !Number.isFinite(Number(c.value))) {
      throw searchQueryInvalid(
        `${c.field}${c.op} needs a number, and ${c.field} must be one: ${SEARCH_FIELDS.filter(f => FIELDS[f].kind === "number").join(", ")}.`,
      );
    }
    return;
  }
  if (kind === "bool" && c.value !== "true" && c.value !== "false") {
    throw searchQueryInvalid(`${c.field} is "true" or "false".`);
  }
  if (kind === "number" && !Number.isFinite(Number(c.value))) {
    throw searchQueryInvalid(`${c.field} is a number.`);
  }
}

/** Whether `node` matches `query`. */
export function matches(query: Query, node: ApiNode): boolean {
  const each = query.clauses.map(c => matchesClause(c, node));
  return query.join === "AND" ? each.every(Boolean) : each.some(Boolean);
}

function matchesClause(c: Clause, node: ApiNode): boolean {
  const field = FIELDS[c.field];
  const v = field.of(node);
  let hit: boolean;
  if (c.value === null) {
    hit = v === null || v === undefined;
  } else if (v === null || v === undefined) {
    hit = false;
  } else if (c.op === "~") {
    hit = String(v).toLowerCase().includes(String(c.value).toLowerCase());
  } else if (c.op === ":") {
    hit =
      field.kind === "number"
        ? Number(v) === Number(c.value)
        : field.kind === "bool"
          ? String(v) === c.value
          : field.kind === "id"
            ? v === c.value
            : String(v).toLowerCase() === String(c.value).toLowerCase();
  } else {
    const a = Number(v);
    const b = Number(c.value);
    hit =
      c.op === ">"
        ? a > b
        : c.op === ">="
          ? a >= b
          : c.op === "<"
            ? a < b
            : a <= b;
  }
  return c.not ? !hit : hit;
}

type Token = {
  kind: "word" | "string" | "number" | "op" | "minus";
  text: string;
  pos: number;
};

function tokenize(text: string): Array<Token> {
  const out: Array<Token> = [];
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (/\s/.test(ch)) {
      i += 1;
    } else if (ch === '"') {
      let s = "";
      let j = i + 1;
      for (; j < text.length && text[j] !== '"'; j++) {
        if (text[j] === "\\" && j + 1 < text.length) {
          j += 1;
        }
        s += text[j];
      }
      if (j >= text.length) {
        throw searchQueryInvalid(
          `The quote at character ${i + 1} isn't closed.`,
        );
      }
      out.push({kind: "string", text: s, pos: i});
      i = j + 1;
    } else if (ch === "-" && /[a-z_]/.test(text[i + 1] ?? "")) {
      out.push({kind: "minus", text: "-", pos: i});
      i += 1;
    } else if (/[:~<>]/.test(ch)) {
      const two = text.slice(i, i + 2);
      const op = two === ">=" || two === "<=" ? two : ch;
      out.push({kind: "op", text: op, pos: i});
      i += op.length;
    } else if (/[-\d]/.test(ch)) {
      const m = /^-?\d+(\.\d+)?/.exec(text.slice(i));
      if (!m) {
        throw searchQueryInvalid(`Unexpected ${ch} at character ${i + 1}.`);
      }
      out.push({kind: "number", text: m[0], pos: i});
      i += m[0].length;
    } else if (/[A-Za-z_]/.test(ch)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(text.slice(i))!;
      out.push({kind: "word", text: m[0], pos: i});
      i += m[0].length;
    } else {
      throw searchQueryInvalid(`Unexpected ${ch} at character ${i + 1}.`);
    }
  }
  return out;
}
