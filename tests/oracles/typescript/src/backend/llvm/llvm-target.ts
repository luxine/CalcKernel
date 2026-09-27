import { spawnSync } from "node:child_process";

export interface LlvmTargetOptions {
  targetTriple?: string;
}

export function normalizeLlvmTargetTriple(targetTriple?: string): string | undefined {
  const normalized = targetTriple?.trim();
  return normalized === "" ? undefined : normalized;
}

export function emitLlvmTargetTriple(targetTriple?: string): string | undefined {
  const normalized = normalizeLlvmTargetTriple(targetTriple);
  return normalized === undefined ? undefined : `target triple = "${escapeLlvmString(normalized)}"`;
}

export function detectNativeLlvmTargetTriple(): string | undefined {
  const result = spawnSync("clang", ["-###", "-x", "c", "-c", "-", "-o", "/dev/null"], {
    encoding: "utf8",
    input: "int ik_target_probe;\n"
  });

  if (result.error) {
    return undefined;
  }

  const output = `${result.stderr ?? ""}\n${result.stdout ?? ""}`;
  const match = /"-triple"\s+"([^"]+)"/.exec(output);
  return match?.[1];
}

function escapeLlvmString(value: string): string {
  return value.replaceAll("\\", "\\\\").replaceAll("\"", "\\\"");
}
