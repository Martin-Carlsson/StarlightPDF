# Command line

Languages without a binding use the command-line tool. The CLI has one subcommand per API operation, plus `--json`, and 1-based inclusive page ranges; each call opens, runs one operation and saves. That leaves nothing extra to design or document, and the conformance suite tests the CLI on every case one call can express; the price is a full save per step, and handle and session behaviour is tested only in the bindings.
