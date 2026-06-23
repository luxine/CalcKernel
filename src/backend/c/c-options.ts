export type OverflowMode = "unchecked" | "checked";

export interface CCodegenOptions {
  overflowMode?: OverflowMode;
}

export const defaultOverflowMode: OverflowMode = "unchecked";
export const checkedOverflowNotImplementedMessage = "checked overflow mode is recognized but not implemented yet";

export function resolveOverflowMode(options: CCodegenOptions = {}): OverflowMode {
  return options.overflowMode ?? defaultOverflowMode;
}

export function assertOverflowModeImplemented(options: CCodegenOptions = {}): void {
  if (resolveOverflowMode(options) === "checked") {
    throw new Error(checkedOverflowNotImplementedMessage);
  }
}
