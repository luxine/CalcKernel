export function buildHyperfineArgs(config, commands, outputPaths) {
  const args = [
    "--warmup",
    String(config.warmup),
    "--runs",
    String(config.runs),
    "--export-json",
    outputPaths.json,
    "--export-markdown",
    outputPaths.markdown
  ];

  for (const command of commands) {
    args.push("--command-name", command.name, command.command);
  }

  return args;
}
