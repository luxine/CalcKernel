import { validateMirModule } from "../../mir/mir-validator.js";
import type { MirBlock, MirFunction, MirInstruction, MirModule, MirPlace, MirTerminator, MirType, MirValue } from "../../mir/mir.js";
import { emitCPrimitiveType, escapeCIncludePath } from "./c-common.js";
import { resolveOverflowMode, type CCodegenOptions } from "./c-options.js";

export interface EmitMirCSourceOptions extends CCodegenOptions {
  headerFileName: string;
}

export function emitMirCSource(module: MirModule, options: EmitMirCSourceOptions): string {
  const validation = validateMirModule(module);
  if (validation.errors.length > 0) {
    throw new Error(`Cannot emit C for invalid MIR: ${validation.errors[0].message}`);
  }

  const overflowMode = resolveOverflowMode(options);
  const lines = [`#include "${escapeCIncludePath(options.headerFileName)}"`];

  for (const func of module.functions) {
    lines.push("", overflowMode === "checked" ? emitCheckedFunction(func) : emitFunction(func));
  }

  return `${lines.join("\n")}\n`;
}

function emitFunction(func: MirFunction): string {
  const prefix = func.exported ? "" : "static ";
  const params = func.params.map((param) => `${emitCType(param.type)} ${param.name}`).join(", ");
  const lines = [`${prefix}${emitCType(func.returnType)} ${func.name}(${params}) {`];
  const declarations = emitDeclarations(func);
  const referencedLabels = collectReferencedLabels(func);

  if (declarations.length > 0) {
    lines.push(...declarations.map((line) => `  ${line}`), "");
  }

  func.blocks.forEach((block, index) => {
    if (index > 0) {
      lines.push("");
    }

    if (referencedLabels.has(block.label)) {
      lines.push(`${block.label}:`);
    }
    for (const instruction of block.instructions) {
      lines.push(`  ${emitInstruction(instruction)}`);
    }
    lines.push(...emitTerminator(block.terminator).map((line) => `  ${line}`));
  });
  lines.push("}");
  return lines.join("\n");
}

function emitCheckedFunction(func: MirFunction): string {
  const prefix = func.exported ? "" : "static ";
  const params = func.params.map((param) => `${emitCType(param.type)} ${param.name}`);
  params.push(`${emitCType(func.returnType)}* ik_return`);

  const lines = [`${prefix}IK_Status ${func.name}(${params.join(", ")}) {`];
  const declarations = emitDeclarations(func);
  const referencedLabels = collectReferencedLabels(func);

  if (functionHasCall(func)) {
    declarations.push("IK_Status ik_status;");
  }

  if (declarations.length > 0) {
    lines.push(...declarations.map((line) => `  ${line}`), "");
  }

  lines.push("  if (ik_return == NULL) {", "    return IK_ERR_NULL_POINTER;", "  }");

  func.blocks.forEach((block, index) => {
    if (index > 0 || declarations.length > 0) {
      lines.push("");
    }

    if (referencedLabels.has(block.label)) {
      lines.push(`${block.label}:`);
    }
    for (const instruction of block.instructions) {
      lines.push(...emitCheckedInstruction(instruction).map((line) => `  ${line}`));
    }
    lines.push(...emitCheckedTerminator(block.terminator).map((line) => `  ${line}`));
  });
  lines.push("}");
  return lines.join("\n");
}

function collectReferencedLabels(func: MirFunction): Set<string> {
  const labels = new Set<string>();

  for (const block of func.blocks) {
    switch (block.terminator.kind) {
      case "jump":
        labels.add(block.terminator.label);
        break;
      case "branch":
        labels.add(block.terminator.thenLabel);
        labels.add(block.terminator.elseLabel);
        break;
      case "return":
        break;
    }
  }

  return labels;
}

function emitDeclarations(func: MirFunction): string[] {
  const lines: string[] = [];
  const seenTemps = new Set<string>();

  for (const local of func.locals) {
    lines.push(`${emitCType(local.type)} ${local.name};`);
  }

  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      const target = instructionTarget(instruction);
      if (target?.kind === "temp" && !seenTemps.has(target.name)) {
        seenTemps.add(target.name);
        lines.push(`${emitCType(target.type)} ${emitValue(target)};`);
      }
    }
  }

  return lines;
}

function functionHasCall(func: MirFunction): boolean {
  return func.blocks.some((block) => block.instructions.some((instruction) => instruction.kind === "call"));
}

function emitInstruction(instruction: MirInstruction): string {
  switch (instruction.kind) {
    case "const_int":
      return `${emitValue(instruction.target)} = ${instruction.value};`;
    case "const_bool":
      return `${emitValue(instruction.target)} = ${instruction.value ? "true" : "false"};`;
    case "move":
      return `${emitValue(instruction.target)} = ${emitValue(instruction.value)};`;
    case "binary":
      return `${emitValue(instruction.target)} = ${emitValue(instruction.left)} ${instruction.op} ${emitValue(instruction.right)};`;
    case "compare":
      return `${emitValue(instruction.target)} = ${emitValue(instruction.left)} ${instruction.op} ${emitValue(instruction.right)};`;
    case "unary":
      return `${emitValue(instruction.target)} = ${instruction.op === "neg" ? "-" : "!"}${emitValue(instruction.operand)};`;
    case "call":
      return `${emitValue(instruction.target)} = ${instruction.functionName}(${instruction.args.map(emitValue).join(", ")});`;
    case "load":
      return `${emitValue(instruction.target)} = ${emitPlace(instruction.place)};`;
    case "store":
      return `${emitPlace(instruction.place)} = ${emitValue(instruction.value)};`;
  }
}

function emitCheckedInstruction(instruction: MirInstruction): string[] {
  switch (instruction.kind) {
    case "const_int":
      return [`${emitValue(instruction.target)} = ${instruction.value};`];
    case "const_bool":
      return [`${emitValue(instruction.target)} = ${instruction.value ? "true" : "false"};`];
    case "move":
      return [`${emitValue(instruction.target)} = ${emitValue(instruction.value)};`];
    case "compare":
      return [`${emitValue(instruction.target)} = ${emitValue(instruction.left)} ${instruction.op} ${emitValue(instruction.right)};`];
    case "unary":
      return emitCheckedUnaryInstruction(instruction);
    case "binary":
      return emitCheckedBinaryInstruction(instruction);
    case "call": {
      const args = [...instruction.args.map(emitValue), `&${emitValue(instruction.target)}`].join(", ");
      return [
        `ik_status = ${instruction.functionName}(${args});`,
        "if (ik_status != IK_OK) {",
        "  return ik_status;",
        "}"
      ];
    }
    case "load":
      return [`${emitValue(instruction.target)} = ${emitPlace(instruction.place)};`];
    case "store":
      return [`${emitPlace(instruction.place)} = ${emitValue(instruction.value)};`];
  }
}

function emitCheckedUnaryInstruction(instruction: Extract<MirInstruction, { kind: "unary" }>): string[] {
  const target = emitValue(instruction.target);
  const operand = emitValue(instruction.operand);

  if (instruction.op === "not") {
    return [`${target} = !${operand};`];
  }

  if (isUnsignedIntegerType(instruction.target.type)) {
    return [
      `if (__builtin_sub_overflow((${emitCType(instruction.target.type)})0, ${operand}, &${target})) {`,
      "  return IK_ERR_OVERFLOW;",
      "}"
    ];
  }

  return [
    `if (${operand} == ${signedMinConstant(instruction.target.type)}) {`,
    "  return IK_ERR_OVERFLOW;",
    "}",
    `${target} = -${operand};`
  ];
}

function emitCheckedBinaryInstruction(instruction: Extract<MirInstruction, { kind: "binary" }>): string[] {
  const target = emitValue(instruction.target);
  const left = emitValue(instruction.left);
  const right = emitValue(instruction.right);

  switch (instruction.op) {
    case "+":
      return emitCheckedOverflowBuiltin("__builtin_add_overflow", left, right, target);
    case "-":
      return emitCheckedOverflowBuiltin("__builtin_sub_overflow", left, right, target);
    case "*":
      return emitCheckedOverflowBuiltin("__builtin_mul_overflow", left, right, target);
    case "/":
    case "%":
      return emitCheckedDivisionOrModulo(instruction.op, left, right, target, instruction.target.type);
  }
}

function emitCheckedOverflowBuiltin(builtin: string, left: string, right: string, target: string): string[] {
  return [
    `if (${builtin}(${left}, ${right}, &${target})) {`,
    "  return IK_ERR_OVERFLOW;",
    "}"
  ];
}

function emitCheckedDivisionOrModulo(operator: "/" | "%", left: string, right: string, target: string, type: MirType): string[] {
  const lines = [
    `if (${right} == 0) {`,
    "  return IK_ERR_DIV_BY_ZERO;",
    "}"
  ];

  if (isSignedIntegerType(type)) {
    lines.push(
      `if (${left} == ${signedMinConstant(type)} && ${right} == -1) {`,
      "  return IK_ERR_OVERFLOW;",
      "}"
    );
  }

  lines.push(`${target} = ${left} ${operator} ${right};`);
  return lines;
}

function emitTerminator(terminator: MirTerminator): string[] {
  switch (terminator.kind) {
    case "return":
      return [`return ${emitValue(terminator.value)};`];
    case "jump":
      return [`goto ${terminator.label};`];
    case "branch":
      return [
        `if (${emitValue(terminator.condition)}) {`,
        `  goto ${terminator.thenLabel};`,
        "} else {",
        `  goto ${terminator.elseLabel};`,
        "}"
      ];
  }
}

function emitCheckedTerminator(terminator: MirTerminator): string[] {
  switch (terminator.kind) {
    case "return":
      return [`*ik_return = ${emitValue(terminator.value)};`, "return IK_OK;"];
    case "jump":
      return [`goto ${terminator.label};`];
    case "branch":
      return [
        `if (${emitValue(terminator.condition)}) {`,
        `  goto ${terminator.thenLabel};`,
        "} else {",
        `  goto ${terminator.elseLabel};`,
        "}"
      ];
  }
}

function instructionTarget(instruction: MirInstruction): MirValue | undefined {
  switch (instruction.kind) {
    case "const_int":
    case "const_bool":
    case "move":
    case "binary":
    case "unary":
    case "compare":
    case "load":
    case "call":
      return instruction.target;
    case "store":
      return undefined;
  }
}

function emitValue(value: MirValue): string {
  switch (value.kind) {
    case "param":
    case "local":
      return value.name;
    case "temp":
      return emitTempName(value.name);
    case "const_int":
      return value.text;
    case "const_bool":
      return value.value ? "true" : "false";
  }
}

function emitPlace(place: MirPlace): string {
  switch (place.kind) {
    case "param":
    case "local":
      return place.name;
    case "index":
      return `${emitPlace(place.base)}[${emitValue(place.index)}]`;
    case "field":
      return `${emitPlace(place.base)}.${place.fieldName}`;
  }
}

function emitTempName(name: string): string {
  const numeric = /^t(\d+)$/.exec(name);
  if (numeric) {
    return `ik_tmp${numeric[1]}`;
  }
  return `ik_tmp_${name.replaceAll(/[^A-Za-z0-9_]/g, "_")}`;
}

function emitCType(type: MirType): string {
  switch (type.kind) {
    case "primitive":
      return emitCPrimitiveType(type.name);
    case "pointer":
      return `${emitCType(type.elementType)}*`;
    case "struct":
      return type.name;
  }
}

function signedMinConstant(type: MirType): string {
  if (type.kind === "primitive" && type.name === "i32") {
    return "INT32_MIN";
  }

  if (type.kind === "primitive" && type.name === "i64") {
    return "INT64_MIN";
  }

  throw new Error("Checked MIR C emission requires a signed integer type for signed overflow checks.");
}

function isSignedIntegerType(type: MirType): boolean {
  return type.kind === "primitive" && (type.name === "i32" || type.name === "i64");
}

function isUnsignedIntegerType(type: MirType): boolean {
  return type.kind === "primitive" && (type.name === "u32" || type.name === "u64");
}
