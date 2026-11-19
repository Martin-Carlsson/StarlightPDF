#!/bin/sh
# Fails on a Cargo.lock package from git or outside the workspace, or a README.md crates.io row that misses or misdescribes a crate or CI tool.
set -eu
export LC_ALL=C
cd "$(dirname "$0")/.."
t=$(printf '\t')

# name manifest, one per line
members=$(find . -name Cargo.toml -not -path './target/*' | sort | while IFS= read -r m; do
  n=$(sed -n 's/^name = "\(.*\)"$/\1/p' "${m}" | head -n 1)
  [ -z "${n}" ] || echo "${n} ${m}"
done)
export members
# "P name version source" per package, "D package dependency" per dependency
lock=$(awk '
  function flush() { if (name != "") print "P", name, version, (src == "" ? "-" : src) }
  /^\[\[package\]\]/ { flush(); name = ""; src = ""; deps = 0 }
  /^name = / { gsub(/"/, "", $3); name = $3 }
  /^version = / { gsub(/"/, "", $3); version = $3 }
  /^source = / { gsub(/"/, "", $3); src = $3 }
  /^dependencies = \[/ { deps = 1; next }
  deps && /^\]/ { deps = 0 }
  deps { s = $0; gsub(/^ *"|",?$/, "", s); print "D", name, s }
  END { flush() }
' Cargo.lock)
bad=$(echo "${lock}" | awk '
  BEGIN { n = split(ENVIRON["members"], a, "\n"); for (i = 1; i <= n; i++) { split(a[i], f, " "); ws[f[1]] = 1 } }
  $1 != "P" { next }
  $4 == "-" && !($2 in ws) { print $2, $3 ": path outside the workspace" }
  $4 != "-" && $4 != "registry+https://github.com/rust-lang/crates.io-index" && $4 != "sparse+https://index.crates.io/" { print $2, $3 ": from " $4 }
')
if [ -n "${bad}" ]; then
  printf 'Cargo.lock: packages must come from crates.io or the workspace:\n%s\n' "${bad}" >&2
  exit 1
fi

# name version of each CI tool, one per line
tools=$(sed -n 's/.*cargo install .*--locked \(.*\)$/\1/p' ci/setup.sh | tr ' ' '\n' | grep @ | tr @ ' ')
# name version license, one per line
lic=$(
  {
    echo "${lock}" | awk '$1 == "P" && $4 != "-" { print $2, $3 }'
    echo "${tools}"
  } | sort -u | while IFS=' ' read -r name version; do
    info=$(cargo info "${name}@${version}" </dev/null) || exit 1
    declared=$(echo "${info}" | sed -n 's/^license: //p' | sed 's|/| OR |g')
    [ -n "${declared}" ] || { echo "${name} ${version}: no license declared" >&2; exit 1; }
    echo "${name} ${version} ${declared}"
  done
)
# key<TAB>license: key is "name", or "name version" when the crate's versions differ in license
keys=$(echo "${lic}" | awk -v OFS="${t}" '
  { n[NR] = $1; v[NR] = $2; l = $0; sub(/^[^ ]+ [^ ]+ /, "", l); lc[NR] = l; if (!((n[NR], l) in seen)) { seen[n[NR], l] = 1; c[n[NR]]++ } }
  END { for (i = 1; i <= NR; i++) print (c[n[i]] > 1 ? n[i] " " v[i] : n[i]), lc[i] }
' | sort -u)
# name<TAB>slug<TAB>license<TAB>reason per crates.io row
rows=$(sed -n '/^### Dependencies$/,$p' README.md | grep '^|.*(https://crates\.io/crates/' | awk -F' [|] ' -v OFS="${t}" '{
  name = substr($1, 3); lic = $2; why = $4; sub(/ \|$/, "", why)
  while (match(lic, /\[[^]]*\]\([^)]*\)/)) { s = substr(lic, RSTART + 1, RLENGTH - 1); sub(/\]\(.*/, "", s); lic = substr(lic, 1, RSTART - 1) s substr(lic, RSTART + RLENGTH) }
  slug = $3; sub(/.*\(https:\/\/crates\.io\/crates\//, "", slug); sub(/[)\/].*/, "", slug)
  print name, slug, lic, why
}')

want=$(echo "${keys}" | cut -f1)
have=$(echo "${rows}" | cut -f1 | sort)
if [ "${want}" != "${have}" ]; then
  echo 'README.md Dependencies: crates.io rows differ from the Cargo.lock packages and CI tools (name, or name version):' >&2
  { echo "${want}"; echo "${have}"; } | sort | uniq -u >&2
  exit 1
fi
echo "${rows}" | while IFS="${t}" read -r name slug row _; do
  [ "${name%% *}" = "${slug}" ] || { echo "README.md Dependencies: ${name}: links crates.io/crates/${slug}" >&2; exit 1; }
  declared=$(echo "${keys}" | awk -F"${t}" -v k="${name}" '$1 == k { print $2 }')
  [ "${row}" = "${declared}" ] || { echo "README.md Dependencies: ${name}: row has '${row}', declared '${declared}'" >&2; exit 1; }
done

echo "${tools}" | while IFS=' ' read -r name version; do
  grep -F "| ${name} | " README.md | grep -qF "(https://crates.io/crates/${name}/${version})" || {
    echo "README.md Dependencies: ${name}: link crates.io/crates/${name}/${version}, as ci/setup.sh installs" >&2
    exit 1
  }
done

c='#[[:space:]]*[^[:space:]]'
echo "${lock}" | awk '
  BEGIN { n = split(ENVIRON["members"], a, "\n"); for (i = 1; i <= n; i++) { split(a[i], f, " "); ws[f[1]] = f[2] } }
  $1 == "P" { ver[$2] = $3 }
  $1 == "D" && ($2 in ws) && !($3 in ws) { d[++k] = ws[$2] " " $3 " " $4 }
  END { for (i = 1; i <= k; i++) { split(d[i], f, " "); print f[1], f[2], (f[3] != "" ? f[3] : ver[f[2]]) } }
' | sort -u | while IFS=' ' read -r manifest dep version; do
  key=$(echo "${keys}" | cut -f1 | grep -xF -e "${dep} ${version}" -e "${dep}" | head -n 1)
  why=$(echo "${rows}" | awk -F"${t}" -v k="${key}" '$1 == k { print $4 }')
  n=$(printf '%s' "${dep}" | sed 's/[-_]/[-_]/g')
  reasons=$(grep -E "^[[:space:]]*\"?${n}\"?[[:space:]]*[.=].*${c}|^\[.*dependencies\.\"?${n}\"?\].*${c}|package[[:space:]]*=[[:space:]]*\"${dep}\".*${c}" "${manifest}" |
    sed 's/^[^#]*#[[:space:]]*//; s/[[:space:]]*$//')
  [ -n "${reasons}" ] || { echo "${manifest}: ${dep}: no reason (add a \`#\` comment on the line)" >&2; exit 1; }
  echo "${reasons}" | while IFS= read -r reason; do
    [ "${reason%.}" = "${why%.}" ] || {
      echo "README.md Dependencies: ${key}: sentence '${why}' differs from the reason '${reason}' in ${manifest}" >&2
      exit 1
    }
  done
done
