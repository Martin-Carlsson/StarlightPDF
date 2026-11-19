#!/bin/sh
# Type-checks every mod against Claude Code's API declarations.
# typescript 7.0.2 (Apache-2.0): the standard type-checker; too large to write here.
set -eu

mods=$(cd "$(dirname "$0")" && pwd)
root=${CLAUDE_CODE_TMPDIR:-/tmp}/claude-$(id -u)/bundled-skills
decl=
for f in "${root}"/*/*/plugin-authoring/types/claude-code.d.ts; do
  if [ -e "${f}" ] && { [ -z "${decl}" ] || [ "${f}" -nt "${decl}" ]; }; then decl=${f}; fi
done
[ -n "${decl}" ] || {
  echo "typecheck.sh: no claude-code.d.ts in ${root}; load the plugin-authoring skill to write it" >&2
  exit 1
}
find "${mods}" \( -name '*.ts' -o -name '*.tsx' \) -not -path '*/node_modules/*' -exec \
  npx -y -p typescript@7.0.2 tsc --noEmit \
  --target es2023 --lib es2023 --types '' --module esnext --moduleResolution bundler \
  --jsx react --jsxFactory h --jsxFragmentFactory Fragment \
  --strict --noUncheckedIndexedAccess --exactOptionalPropertyTypes --noImplicitReturns \
  --noFallthroughCasesInSwitch --noImplicitOverride --noPropertyAccessFromIndexSignature \
  --noUnusedLocals --noUnusedParameters --allowUnreachableCode false --allowUnusedLabels false \
  --noUncheckedSideEffectImports --verbatimModuleSyntax --isolatedModules --erasableSyntaxOnly \
  --declaration --isolatedDeclarations \
  "${decl}" {} +
