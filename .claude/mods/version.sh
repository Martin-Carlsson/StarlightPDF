#!/bin/sh
# Fails for each plugin whose files differ from BASE while its version does not. Usage: version.sh BASE
set -eu

base=$1
git rev-parse -q --verify "${base}^{commit}" >/dev/null || {
  echo "version.sh: no ${base}" >&2
  exit 1
}
top=$(git rev-parse --show-toplevel)
cd "${top}"
index=$(mktemp)
trap 'rm -f "${index}"' EXIT
git_index=$(git rev-parse --git-path index)
cp "${git_index}" "${index}" 2>/dev/null || rm -f "${index}"
GIT_INDEX_FILE=${index} git add -A
tree=$(GIT_INDEX_FILE=${index} git write-tree)
version='s/.*"version": *"\([^"]*\)".*/\1/p'
sources=$(sed -n 's/.*"source": *"\.\/\([^"]*\)".*/\1/p' .claude-plugin/marketplace.json)
failed=0
while IFS= read -r dir; do
  old=$(git rev-parse -q --verify "${base}:${dir}") || continue
  new=$(git rev-parse -q --verify "${tree}:${dir}") || new=
  [ "${old}" = "${new}" ] && continue
  manifest=${dir}/.claude-plugin/plugin.json
  was=$(git show "${base}:${manifest}" | sed -n "${version}")
  now=$(sed -n "${version}" "${manifest}")
  if [ "${was}" = "${now}" ]; then
    echo "version.sh: ${dir} changed but its version is still ${now}; raise it" >&2
    failed=1
  fi
done <<END
${sources}
END
exit "${failed}"
