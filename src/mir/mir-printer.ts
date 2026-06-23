import type {
  MirBinaryOp,
  MirCompareOp,
  MirFunction,
  MirInstruction,
  MirModule,
  MirPlace,
  MirStruct,
  MirTerminator,
  MirType,
  MirUnaryOp,
  MirValue
} from "./mir.js";

export function printMirModule(module: MirModule): string {
  const parts: string[] = [];

  for (const struct of module.structs) {
    parts.push(printMirStruct(struct));
  }

  for (const func of module.functions) {
    parts.push(printMirFunction(func));
  }

  return parts.length === 0 ? "" : `${parts.join("\n\n")}\n`;
}

function printMirStruct(struct: MirStruct): string {
  const lines = [`struct ${struct.name} {`];
  for (const field of struct.fields) {
    lines.push(`  ${field.name}: ${printMirType(field.type)}`);
  }
  lines.push("}");
  return lines.join("\n");
}

function printMirFunction(func: MirFunction): string {
  const exported = func.exported ? "export " : "";
  const params = func.params.map((param) => `${param.name}: ${printMirType(param.type)}`).join(", ");
  const lines = [`${exported}fn ${func.name}(${params}) -> ${printMirType(func.returnType)} {`];

  if (func.locals.length > 0) {
    for (const local of func.locals) {
      lines.push(`  local ${local.name}: ${printMirType(local.type)}`);
    }
    if (func.blocks.length > 0) {
      lines.push("");
    }
  }

  func.blocks.forEach((block, index) => {
    if (index > 0) {
      lines.push("");
    }

    lines.push(`${block.label}:`);
    for (const instruction of block.instructions) {
      lines.push(`  ${printMirInstruction(instruction)}`);
    }
    lines.push(`  ${printMirTerminator(block.terminator)}`);
  });

  lines.push("}");
  return lines.join("\n");
}

function printMirInstruction(instruction: MirInstruction): string {
  switch (instruction.kind) {
    case "const_int":
      return `${printMirValue(instruction.target)}: ${printMirType(instruction.target.type)} = const_int ${instruction.value}`;
    case "const_bool":
      return `${printMirValue(instruction.target)}: ${printMirType(instruction.target.type)} = const_bool ${instruction.value ? "true" : "false"}`;
    case "move":
      return `${printMirValue(instruction.target)}: ${printMirType(instruction.target.type)} = move ${printMirValue(instruction.value)}`;
    case "binary":
      return `${printMirValue(instruction.target)}: ${printMirType(instruction.target.type)} = ${printBinaryOp(instruction.op)} ${printMirValue(instruction.left)}, ${printMirValue(instruction.right)}`;
    case "unary":
      return `${printMirValue(instruction.target)}: ${printMirType(instruction.target.type)} = ${printUnaryOp(instruction.op)} ${printMirValue(instruction.operand)}`;
    case "compare":
      return `${printMirValue(instruction.target)}: ${printMirType(instruction.target.type)} = ${printCompareOp(instruction.op)} ${printMirValue(instruction.left)}, ${printMirValue(instruction.right)}`;
    case "load":
      return `${printMirValue(instruction.target)}: ${printMirType(instruction.target.type)} = load ${printMirPlace(instruction.place)}`;
    case "store":
      return `store ${printMirPlace(instruction.place)}, ${printMirValue(instruction.value)}`;
    case "call":
      return `${printMirValue(instruction.target)}: ${printMirType(instruction.target.type)} = call ${instruction.functionName}(${instruction.args.map(printMirValue).join(", ")})`;
  }
}

function printMirTerminator(terminator: MirTerminator): string {
  switch (terminator.kind) {
    case "return":
      return `return ${printMirValue(terminator.value)}`;
    case "jump":
      return `jump ${terminator.label}`;
    case "branch":
      return `branch ${printMirValue(terminator.condition)}, ${terminator.thenLabel}, ${terminator.elseLabel}`;
  }
}

function printMirValue(value: MirValue): string {
  switch (value.kind) {
    case "param":
    case "local":
      return value.name;
    case "temp":
      return `%${value.name}`;
    case "const_int":
      return value.text;
    case "const_bool":
      return value.value ? "true" : "false";
  }
}

function printMirPlace(place: MirPlace): string {
  switch (place.kind) {
    case "param":
    case "local":
      return place.name;
    case "index":
      return `index(${printMirPlace(place.base)}, ${printMirValue(place.index)})`;
    case "field":
      return `field(${printMirPlace(place.base)}, ${place.fieldName})`;
  }
}

export function printMirType(type: MirType): string {
  switch (type.kind) {
    case "primitive":
      return type.name;
    case "pointer":
      return `ptr<${printMirType(type.elementType)}>`;
    case "struct":
      return type.name;
  }
}

function printBinaryOp(op: MirBinaryOp): string {
  switch (op) {
    case "+":
      return "add";
    case "-":
      return "sub";
    case "*":
      return "mul";
    case "/":
      return "div";
    case "%":
      return "mod";
  }
}

function printCompareOp(op: MirCompareOp): string {
  switch (op) {
    case "==":
      return "eq";
    case "!=":
      return "ne";
    case "<":
      return "lt";
    case "<=":
      return "le";
    case ">":
      return "gt";
    case ">=":
      return "ge";
  }
}

function printUnaryOp(op: MirUnaryOp): string {
  return op;
}
