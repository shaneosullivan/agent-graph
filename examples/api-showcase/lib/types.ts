// The Agent Graph API's objects, as its spec (site/openapi.json) describes
// them. A field ending `_id` is always an id; the field without it (`parent`)
// is the object, present only when it's been expanded.

export type NodeState =
  "working" | "input_required" | "idle" | "completed" | "failed" | "canceled";

export type List<T> = {
  object: "list";
  url: string;
  as_of_event_id: string | null;
  has_more: boolean;
  data: Array<T>;
};

export type SearchResult<T> = {
  object: "search_result";
  url: string;
  as_of_event_id: string | null;
  has_more: boolean;
  next_page: string | null;
  total_count: number;
  data: Array<T>;
};

export type Graph = {
  id: string;
  object: "graph";
  as_of_event_id: string | null;
  source: string;
  url: string;
  created: number;
  last_event: number;
  retention: {
    first_event_id: string | null;
    last_event_id: string | null;
    first_event_created: number | null;
    last_event_created: number | null;
    event_count: number;
  };
  counts: {
    nodes: number;
    sessions: number;
    agents: number;
    stale: number;
    blocked: number;
    by_state: Record<NodeState, number>;
  };
};

export type Task = {
  id: string;
  text: string;
  active_text: string | null;
  status: "pending" | "in_progress" | "completed";
};

export type Spawn = {
  call_id: string;
  kind: "agent" | "session";
  agent_type: string | null;
  purpose: string | null;
  background: boolean;
  child_id: string | null;
  returned: boolean;
  requested: number;
};

export type Wait = {
  id: string;
  kind: "spawn" | "explicit";
  on_id: string | null;
  reason: string | null;
  open: boolean;
  started: number;
  ended: number | null;
};

export type Message = {
  id: string;
  direction: "sent" | "received";
  peer_id: string | null;
  peer_name: string | null;
  summary: string | null;
  body: string | null;
  created: number;
};

export type AgentNode = {
  id: string;
  object: "node";
  as_of_event_id: string | null;
  graph_id: string;
  provider_ref: string;
  provider: string;
  kind: "session" | "agent";
  parent_id: string | null;
  root_id: string;
  session_id: string;
  requested_by_id: string | null;
  depth: number;
  state: NodeState;
  stale: boolean;
  attention: string | null;
  title: string | null;
  summary: string | null;
  headline: string | null;
  purpose: string | null;
  agent_type: string | null;
  background: boolean | null;
  cwd: string | null;
  created: number;
  ended: number | null;
  last_event: number;
  child_count: number;
  descendant_count: number;
  task_counts: {
    total: number;
    pending: number;
    in_progress: number;
    completed: number;
    open: number;
  };
  blocked: {
    on_ids: Array<string>;
    on?: Array<AgentNode>;
    starting: number;
    node_count: number;
    open_tasks: number;
    cycle: boolean;
  } | null;
  // Present only when expanded.
  parent?: AgentNode;
  root?: AgentNode;
  session?: AgentNode;
  children?: List<AgentNode>;
  descendants?: List<AgentNode>;
  tasks?: Array<Task>;
  spawns?: Array<Spawn>;
  waits?: Array<Wait>;
  messages?: Array<Message>;
};

export type NodeChange = {
  node_id: string;
  created: boolean;
  fields: Record<string, {from: unknown; to: unknown}>;
};

export type AgentEvent = {
  id: string;
  object: "event";
  type: string;
  created: number;
  graph_id: string;
  node_id: string;
  source: {
    provider: string;
    provider_version: string | null;
    adapter: string | null;
  };
  payload: Record<string, unknown>;
  changes: Array<NodeChange>;
  node?: AgentNode;
};

export type ApiErrorBody = {
  error: {
    type: string;
    code: string;
    message: string;
    param: string | null;
    doc_url: string;
  };
};
