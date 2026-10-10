// Cursor pagination (site/openapi.json, "Pagination"): a page after an
// object (`starting_after`), or before one (`ending_before`), in the list's
// own order.

import {parameterInvalid} from "./errors";
import {DEFAULT_LIMIT} from "./query";

export type Cursors = {
  limit?: number;
  starting_after?: string;
  ending_before?: string;
};

/**
 * The page of `items` (already in the list's order) that `cursors` asks
 * for, and whether there are more past it: after `starting_after`, before
 * `ending_before`, or from the start. A cursor that isn't in the list is
 * refused: it was from another list, or another moment.
 */
export function paginate<T>(
  items: ReadonlyArray<T>,
  idOf: (item: T) => string,
  cursors: Cursors,
): {data: Array<T>; has_more: boolean} {
  const limit = cursors.limit ?? DEFAULT_LIMIT;
  const {starting_after: after, ending_before: before} = cursors;
  if (after !== undefined && before !== undefined) {
    throw parameterInvalid(
      "ending_before",
      "Give starting_after or ending_before, not both.",
    );
  }
  const at = (id: string, param: string) => {
    const i = items.findIndex(item => idOf(item) === id);
    if (i < 0) {
      throw parameterInvalid(
        param,
        `${id} isn't in this list (pass the same filters and as_of as the page it came from).`,
      );
    }
    return i;
  };
  if (before !== undefined) {
    const end = at(before, "ending_before");
    const start = Math.max(0, end - limit);
    return {data: items.slice(start, end), has_more: start > 0};
  }
  const start = after === undefined ? 0 : at(after, "starting_after") + 1;
  return {
    data: items.slice(start, start + limit),
    has_more: start + limit < items.length,
  };
}
