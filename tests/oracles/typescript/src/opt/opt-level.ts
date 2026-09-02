export type OptimizationLevel = 0 | 1 | 2 | 3;

export interface OptimizationOptions {
  optLevel?: OptimizationLevel;
}

export const defaultOptimizationLevel: OptimizationLevel = 0;

export function resolveOptimizationLevel(options: OptimizationOptions = {}): OptimizationLevel {
  return options.optLevel ?? defaultOptimizationLevel;
}

export function parseOptimizationLevel(value: string): OptimizationLevel | undefined {
  switch (value) {
    case "0":
      return 0;
    case "1":
      return 1;
    case "2":
      return 2;
    case "3":
      return 3;
    default:
      return undefined;
  }
}
