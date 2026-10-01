# Sourced, not run: the steps and timings the build and test scripts print
# (scripts/test-all.sh, scripts/ci-slow-checks.sh, scripts/build-local.sh),
# and "lanes": commands run side by side, each into its own log.
#
# A lane's output isn't mixed into the others'. While lanes run, a line
# says which have started and, every LANE_STATUS_EVERY seconds (30), each
# one's latest line, if it's changed. Once they've all finished, each one's
# whole output follows, the ones that failed last, so the failure ends the
# log, where Chofter CI's "What broke", the evidence it attaches to a pull
# request and its comment on one all look.
#
#   lane "site" npm run build
#   lane "release builds" bash scripts/build-local.sh
#   lanes_wait   # fails if any lane did

# 75 seconds as 1m15s.
duration() {
  local s=$1
  if [ "$s" -ge 60 ]; then printf '%dm%02ds' $((s / 60)) $((s % 60)); else printf '%ds' "$s"; fi
}

# Prints the command, runs it, and says how long it took, or that it failed.
step() {
  printf '\n==> %s\n' "$*"
  local start=$SECONDS code=0
  "$@" || code=$?
  if [ "$code" = 0 ]; then
    printf '<== took %s: %s\n' "$(duration $((SECONDS - start)))" "$*"
  else
    printf '<== exit %s after %s: %s\n' "$code" "$(duration $((SECONDS - start)))" "$*"
  fi
  return "$code"
}

_lanes_dir=""
_lane_names=()
_lane_pids=()
_lane_starts=()

_lanes_stop() {
  local pid
  for pid in "${_lane_pids[@]}"; do
    # The lane's own commands first: a subshell's death doesn't stop them.
    command -v pkill >/dev/null 2>&1 && pkill -P "$pid" 2>/dev/null || true
    kill "$pid" 2>/dev/null || true
  done
  [ -n "$_lanes_dir" ] && rm -rf "$_lanes_dir"
  return 0
}

# Starts a command in the background, its output kept apart.
lane() {
  local name=$1
  shift
  if [ -z "$_lanes_dir" ]; then
    _lanes_dir="$(mktemp -d)"
    # A cancelled run stops what its lanes started. (Not an EXIT trap: the
    # scripts have their own, and lanes_wait stops them anyway.)
    trap '_lanes_stop; exit 130' INT TERM
  fi
  # Named so a lane's own commands (a shell function's loop, say) can't
  # change them: bash's variables reach into what a function calls.
  local _lane_out="$_lanes_dir/${#_lane_names[@]}"
  (
    set +e
    "$@" >"$_lane_out.log" 2>&1
    # Renamed into place, so it's never read half written.
    echo $? >"$_lane_out.code.new" && mv "$_lane_out.code.new" "$_lane_out.code"
  ) &
  _lane_names+=("$name")
  _lane_pids+=($!)
  _lane_starts+=("$SECONDS")
  printf '\n==> [%s] started: %s\n' "$name" "$*"
  printf '    (its output follows once every lane has finished)\n'
}

# A lane's latest line of output, without colour codes or progress redraws.
_lane_latest() {
  tail -n 20 "$1" 2>/dev/null |
    tr '\r' '\n' |
    sed 's/\x1b\[[0-9;]*[A-Za-z]//g' |
    grep -v -e '^[[:space:]]*$' -e '^[[:space:]]*\(\.\.\.\|---\)$' -e '^    (its output follows' |
    tail -n 1 |
    cut -c 1-200 || true
}

# Waits for every lane, saying how they're getting on, then prints each
# one's output. Fails if any lane did.
lanes_wait() {
  local every=${LANE_STATUS_EVERY:-30}
  local n=${#_lane_names[@]} i
  local -a done_at=() last=() codes=()
  local remaining=$n next_status=$((SECONDS + every))
  while [ "$remaining" -gt 0 ]; do
    sleep 1
    for ((i = 0; i < n; i++)); do
      [ -n "${done_at[i]:-}" ] && continue
      [ -f "$_lanes_dir/$i.code" ] || continue
      codes[i]=$(cat "$_lanes_dir/$i.code")
      done_at[i]=$SECONDS
      remaining=$((remaining - 1))
      local took
      took=$(duration $((SECONDS - _lane_starts[i])))
      if [ "${codes[i]}" = 0 ]; then
        printf '==> [%s] passed in %s\n' "${_lane_names[i]}" "$took"
      else
        printf '==> [%s] failed (exit %s) after %s\n' "${_lane_names[i]}" "${codes[i]}" "$took"
      fi
    done
    if [ "$remaining" -gt 0 ] && [ "$SECONDS" -ge "$next_status" ]; then
      next_status=$((SECONDS + every))
      for ((i = 0; i < n; i++)); do
        [ -n "${done_at[i]:-}" ] && continue
        local latest
        latest=$(_lane_latest "$_lanes_dir/$i.log")
        [ -n "$latest" ] && [ "$latest" != "${last[i]:-}" ] || continue
        last[i]=$latest
        printf '    [%s] %s in: %s\n' "${_lane_names[i]}" \
          "$(duration $((SECONDS - _lane_starts[i])))" "$latest"
      done
    fi
  done
  wait

  # Each lane's output, in the order they were started, but the failed
  # ones last.
  local failed=() summary="" order=()
  for ((i = 0; i < n; i++)); do [ "${codes[i]}" = 0 ] && order+=("$i"); done
  for ((i = 0; i < n; i++)); do [ "${codes[i]}" = 0 ] || order+=("$i"); done
  for i in "${order[@]}"; do
    local took
    took=$(duration $((done_at[i] - _lane_starts[i])))
    printf '\n==> [%s] its output (%s, exit %s)\n' "${_lane_names[i]}" "$took" "${codes[i]}"
    cat "$_lanes_dir/$i.log"
    if [ "${codes[i]}" = 0 ]; then
      summary+="${summary:+; }${_lane_names[i]} passed ($took)"
    else
      failed+=("${_lane_names[i]}")
      summary+="${summary:+; }${_lane_names[i]} failed, exit ${codes[i]} ($took)"
    fi
  done
  printf '\n==> Lanes: %s\n' "$summary"
  _lanes_stop
  trap - INT TERM
  _lanes_dir="" _lane_names=() _lane_pids=() _lane_starts=()
  if [ ${#failed[@]} -gt 0 ]; then
    printf 'Failed: %s\n' "$(IFS=,; echo "${failed[*]}" | sed 's/,/, /g')"
    return 1
  fi
}

# Where cargo builds: CARGO_TARGET_DIR (Chofter CI keeps one between runs,
# scripts/ci-setup.sh), else target/.
cargo_target_dir() {
  printf '%s' "${CARGO_TARGET_DIR:-target}"
}
