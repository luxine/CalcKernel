import type { FunctionDeclaration, StructDeclaration } from "../parser/ast.js";
import type { IntKernelType } from "./types.js";

export interface StructSymbol {
  name: string;
  declaration: StructDeclaration;
  fields: Map<string, IntKernelType>;
}

export interface FunctionSymbol {
  name: string;
  declaration: FunctionDeclaration;
  params: IntKernelType[];
  returnType: IntKernelType;
}

export interface VariableSymbol {
  name: string;
  type: IntKernelType;
}

export class SymbolTable {
  readonly structs = new Map<string, StructSymbol>();
  readonly functions = new Map<string, FunctionSymbol>();
}

export class Scope {
  private readonly variables = new Map<string, VariableSymbol>();

  constructor(readonly parent: Scope | null = null) {}

  declare(variable: VariableSymbol): boolean {
    if (this.variables.has(variable.name)) {
      return false;
    }

    this.variables.set(variable.name, variable);
    return true;
  }

  lookup(name: string): VariableSymbol | null {
    return this.variables.get(name) ?? this.parent?.lookup(name) ?? null;
  }
}
