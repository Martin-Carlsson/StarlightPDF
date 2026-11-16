#!/bin/sh
# Fails naming each workflow, job or step without a name of its own. Usage: workflows.sh FILE...
# Reads the repository's fixed indentation: jobs at 2 spaces, their keys at 4, steps at 6.
set -eu

bad=0
for f; do
  awk -v f="$f" '
    function value(v) {
      v = $0
      sub(/^ *(- )?name: */, "", v)
      if (v ~ /^".*"$/ || v ~ /^\047.*\047$/) v = substr(v, 2, length(v) - 2)
      return v
    }
    function fail(m) { print f ": " m > "/dev/stderr"; bad = 1 }
    function end_step() {
      if (step && !named) fail("job " job ": name the step at line " step)
      step = 0
    }
    function end_steps() {
      end_step()
      if (steps && !nsteps) fail("job " job ": put its steps at 6 spaces")
      steps = nsteps = 0
    }
    function end_job() {
      end_steps()
      if (job != "" && !jobnamed) fail("job " job ": give it a name other than its id")
      job = ""
    }
    function end_jobs() {
      end_job()
      if (top == "jobs:" && !njobs) fail("put its jobs at 2 spaces")
    }
    BEGIN { base = f; sub(/.*\//, "", base); stem = base; sub(/\.[^.]*$/, "", stem) }
    { sub(/ +#.*/, ""); sub(/ +$/, "") }
    /^ *(#|$)/ { next }
    { match($0, /^ */); n = RLENGTH }
    n == 0 {
      end_jobs()
      top = $0
      if (/^jobs: /) fail("put its jobs on their own lines")
      if ($0 ~ /^name:/) { v = value(); named_wf = v != "" && v != base && v != stem }
    }
    n == 2 && top == "jobs:" {
      end_job()
      job = $0
      sub(/^ */, "", job)
      sub(/:.*/, "", job)
      jobnamed = 0
      njobs++
    }
    n == 4 && job != "" {
      end_steps()
      steps = $0 == "    steps:"
      if (/^    steps: /) fail("job " job ": put its steps on their own lines")
      if ($0 ~ /^    name:/) { v = value(); jobnamed = v != "" && v != job }
    }
    steps && n == 6 && /^      -( |$)/ { end_step(); step = NR; nsteps++; named = /^      - name:/ && value() != "" }
    steps && n == 8 && /^        name:/ { named = value() != "" }
    END {
      end_jobs()
      if (!named_wf) fail("give the workflow a name other than its file name")
      exit bad
    }
  ' "$f" || bad=1
done
exit "$bad"
