#!/bin/sh
# Fails when a stable rustc, clippy or rustdoc lint is neither denied nor allowed with a reason, a
# member does not inherit the workspace lints, the core or Python root does not forbid unsafe code,
# or a file other than tests/memory/src/lib.rs names it except to forbid it or as the root deny.
set -eu
cd "$(dirname "$0")/.."
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
: > "${t}/lints"
: > "${t}/groups"

# Lint tables "name default" and group tables "group member"; names use `_`.
for tool in rustc clippy-driver rustdoc; do
  "${tool}" -W help | tr -d '\r' | tr '-' '_' | awk -v l="${t}/lints" -v g="${t}/groups" '
    /^Lint checks/ { mode = "l"; next }
    /^Lint groups/ { mode = "g"; next }
    mode == "l" && $2 ~ /^(allow|warn|deny|forbid)$/ { print $1, $2 >> l }
    mode == "g" && NF >= 2 && $1 !~ /^(name|warnings|Lint|_+)$/ {
      for (i = 2; i <= NF; i++) { m = $i; sub(/,$/, "", m); print $1, m >> g }
    }'
done
if ! { grep -q '^unsafe_code ' "${t}/lints" && grep -q '^clippy::unwrap_used ' "${t}/lints" &&
  grep -q '^rustdoc::bare_urls ' "${t}/lints" && grep -q '^clippy::pedantic ' "${t}/groups"; }; then
  echo "lints.sh: cannot parse the lint tables" >&2
  exit 2
fi

# Unstable lints cannot be named: the compiler reports them as unknown.
{
  echo '#![warn(unknown_lints)]'
  awk '{ print "#![warn(" $1 ")]" }' "${t}/lints"
} > "${t}/probe.rs"
{
  clippy-driver --crate-type lib --emit=metadata --out-dir "${t}" "${t}/probe.rs" 2>&1 || true
  rustdoc --out-dir "${t}/doc" "${t}/probe.rs" 2>&1 || true
} | tr -d '\r' | sed -n "s/^warning.*unknown lint: \`\(.*\)\`\$/\1/p" > "${t}/unstable"
grep -qx test_unstable_lint "${t}/unstable" || { echo "lints.sh: cannot detect unstable lints" >&2; exit 2; }

# Lints set in the manifest: "name level reason"; reason is 1 when a `#` comment follows.
awk '
  /^\[/ { s = ""; if ($0 ~ /^\[workspace\.lints\.(rust|clippy|rustdoc)\]/) { s = $0; sub(/^\[workspace\.lints\./, "", s); sub(/\].*/, "", s) } next }
  s != "" && /^[A-Za-z0-9_-]+[ \t]*=/ {
    n = $1; sub(/=.*/, "", n); gsub(/-/, "_", n)
    if (s != "rust") n = s "::" n
    if (n == "clippy::all") n = "clippy::default"
    lvl = "?"
    if (match($0, /"(allow|warn|deny|forbid|expect)"/)) lvl = substr($0, RSTART + 1, RLENGTH - 2)
    print n, lvl, ($0 ~ /#[ \t]*[^ \t]/) ? 1 : 0
  }' Cargo.toml > "${t}/set"

# A lint is covered when set to deny or forbid (a group covers its members), allowed with a reason,
# denied by default, or warned by default while `warnings` is denied.
awk -v lf="${t}/lints" -v gf="${t}/groups" -v uf="${t}/unstable" -v sf="${t}/set" '
  FILENAME == lf { dflt[$1] = $2; next }
  FILENAME == gf { grp[$1] = 1; mem[$1] = mem[$1] " " $2; next }
  FILENAME == uf { unstable[$1] = 1; next }
  FILENAME == sf { lvl[$1] = $2; why[$1] = $3; next }
  END {
    for (e in lvl)
      if (lvl[e] == "deny" || lvl[e] == "forbid") {
        cov[e] = 1
        n = split(mem[e], m, " ")
        for (i = 1; i <= n; i++) cov[m[i]] = 1
      } else if (lvl[e] == "allow" && !(e in grp) && why[e]) cov[e] = 1
      else print "invalid: " e " = " lvl[e] (lvl[e] == "allow" && !(e in grp) ? " without reason" : "")
    for (l in dflt) {
      if (l in unstable || l in cov || dflt[l] == "deny") continue
      if (dflt[l] == "warn" && lvl["warnings"] == "deny") continue
      print "missing: " l " (" dflt[l] ")"
    }
  }' "${t}/lints" "${t}/groups" "${t}/unstable" "${t}/set" | sort > "${t}/problems"

if [ -s "${t}/problems" ]; then
  cat "${t}/problems" >&2
  exit 1
fi

for f in core/src/lib.rs bindings/python/src/lib.rs; do
  grep -qxF '#![forbid(unsafe_code)]' "${f}" || { echo "${f}: add #![forbid(unsafe_code)]" >&2; exit 1; }
done
hits=$(git grep -nE --untracked 'unsafe[_-]code' -- ':!ci/lints.sh' ':!tests/memory/src/lib.rs') || [ $? -eq 1 ]
bad=$(printf '%s\n' "${hits}" | grep -vxE '[^:]+:[0-9]+:#!\[forbid\(unsafe_code\)\]|Cargo\.toml:[0-9]+:unsafe_code = "deny"') ||
  [ $? -eq 1 ]
if [ -n "${bad}" ]; then
  printf '%s\n%s\n' "${bad}" 'lints.sh: outside tests/memory/src/lib.rs, name unsafe code only to forbid it' >&2
  exit 1
fi

members=$(sed -n 's/^members = \[\(.*\)\]$/\1/p' Cargo.toml | tr -d '",')
[ -n "${members}" ] || { echo "lints.sh: cannot parse the workspace members" >&2; exit 2; }
for m in ${members}; do
  awk '/^\[/ { s = $0 } s == "[lints]" && $0 == "workspace = true" { ok = 1 } END { exit !ok }' "${m}/Cargo.toml" ||
    { echo "${m}/Cargo.toml: add [lints] workspace = true" >&2; exit 1; }
done
