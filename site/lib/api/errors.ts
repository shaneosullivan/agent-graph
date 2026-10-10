// The API's errors (site/openapi.json, "Errors"): one shape for all of
// them, with a type, a code to branch on, and the parameter it's about.

export type ErrorType =
  | "invalid_request_error"
  | "authentication_error"
  | "permission_error"
  | "rate_limit_error"
  | "api_error";

/** An error to answer with: its status, and what goes in its `error`. */
export class ApiError extends Error {
  readonly status: number;
  readonly type: ErrorType;
  readonly code: string;
  readonly param: string | null;
  /** More it says, by code: `first_event_id`, `last_event_id`. */
  readonly extra: Record<string, string>;
  /** Headers to send with it: `Retry-After`. */
  readonly headers: Record<string, string>;

  constructor(
    status: number,
    type: ErrorType,
    code: string,
    message: string,
    param: string | null = null,
    extra: Record<string, string> = {},
    headers: Record<string, string> = {},
  ) {
    super(message);
    this.status = status;
    this.type = type;
    this.code = code;
    this.param = param;
    this.extra = extra;
    this.headers = headers;
  }

  /** Its body, with where it's explained (`docs`, the docs' own address). */
  body(docs: string): {error: Record<string, unknown>} {
    return {
      error: {
        type: this.type,
        code: this.code,
        message: this.message,
        param: this.param,
        doc_url: `${docs}#errors`,
        ...this.extra,
      },
    };
  }
}

const invalid = (code: string, message: string, param: string | null = null) =>
  new ApiError(400, "invalid_request_error", code, message, param);

export const parameterInvalid = (param: string, message: string) =>
  invalid("parameter_invalid", message, param);

export const parameterMissing = (param: string) =>
  invalid("parameter_missing", `Missing required parameter: ${param}.`, param);

export const parameterUnknown = (param: string) =>
  invalid("parameter_unknown", `Received unknown parameter: ${param}.`, param);

export const expandError = (code: string, message: string) =>
  invalid(code, message, "expand");

export const searchQueryInvalid = (message: string) =>
  invalid("search_query_invalid", message, "query");

export const versionInvalid = (version: string) =>
  invalid(
    "version_invalid",
    `${JSON.stringify(version)} isn't an API version. This API's version is ${API_VERSION}.`,
  );

export const resourceMissing = (what: string, id: string, param: string) =>
  new ApiError(
    404,
    "invalid_request_error",
    "resource_missing",
    `No such ${what}: ${id}.`,
    param,
  );

export const notYetCreated = (node: string, event: string) =>
  new ApiError(
    404,
    "invalid_request_error",
    "node_not_yet_created",
    `The node ${node} is created by ${event}, after as_of.`,
    "as_of",
    {first_event_id: event},
  );

export const outsideRetention = (
  first: string,
  last: string,
  param = "as_of",
) =>
  new ApiError(
    410,
    "invalid_request_error",
    "outside_retention_window",
    `This graph holds events from ${first} to ${last}.`,
    param,
    {first_event_id: first, last_event_id: last},
  );

export const apiKeyMissing = () =>
  new ApiError(
    401,
    "authentication_error",
    "api_key_missing",
    "No API key was sent. Send it as a bearer token (Authorization: Bearer ag_sk_live_…) or as the username of HTTP Basic auth.",
  );

export const apiKeyInvalid = (shown: string) =>
  new ApiError(
    401,
    "authentication_error",
    "api_key_invalid",
    `Invalid API key: ${shown}.`,
  );

export const apiKeyRestricted = (graph: string) =>
  new ApiError(
    403,
    "permission_error",
    "api_key_restricted",
    `This restricted key can't read ${graph}.`,
  );

export const rateLimited = (wait: number) =>
  new ApiError(
    429,
    "rate_limit_error",
    "rate_limit",
    `Too many requests. Try again in ${wait} second${wait === 1 ? "" : "s"}.`,
    null,
    {},
    {"Retry-After": String(wait)},
  );

export const apiFailed = () =>
  new ApiError(
    500,
    "api_error",
    "api_error",
    "Something went wrong. Try again, and include the Request-Id if it keeps happening.",
  );

/** The API's version: the date of the last change that could break an integration. */
export const API_VERSION = "2026-10-09";

/** Every version there's been, oldest first. */
export const API_VERSIONS: ReadonlyArray<string> = [API_VERSION];
