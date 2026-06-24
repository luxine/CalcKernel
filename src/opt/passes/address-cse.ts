import { printMirType } from "../../mir/mir-printer.js";
import type { MirInstruction, MirPlace, MirValue } from "../../mir/mir.js";
import type { MirPass } from "../mir-pass.js";

interface AddressEntry {
  pointer: MirValue;
  dependencies: Set<string>;
}

export const addressCsePass: MirPass = {
  name: "address-cse",
  run(module, context) {
    if (context.targetBackend !== "c" && context.targetBackend !== "wasm") {
      return { changed: false };
    }

    let changed = false;

    for (const func of module.functions) {
      const allocator = createAddressTempAllocator(func);

      for (const block of func.blocks) {
        const addresses = new Map<string, AddressEntry>();
        const nextInstructions: MirInstruction[] = [];

        for (const instruction of block.instructions) {
          if (instruction.kind === "call") {
            addresses.clear();
            nextInstructions.push(instruction);
            continue;
          }

          const inserted: MirInstruction[] = [];
          const rewritten = rewriteInstruction(instruction, addresses, allocator, inserted);
          if (inserted.length > 0 || rewritten !== instruction) {
            changed = true;
          }

          nextInstructions.push(...inserted, rewritten);

          if (instruction.kind === "store") {
            addresses.clear();
            continue;
          }

          const target = instructionTarget(rewritten);
          if (target?.kind === "local" || target?.kind === "param") {
            invalidateDependency(addresses, dependencyKey(target));
          }
        }

        block.instructions = nextInstructions;
      }
    }

    return { changed };
  }
};

function rewriteInstruction(
  instruction: MirInstruction,
  addresses: Map<string, AddressEntry>,
  allocator: (elementType: MirPlace["type"]) => MirValue,
  inserted: MirInstruction[]
): MirInstruction {
  switch (instruction.kind) {
    case "load":
      return { ...instruction, place: rewritePlace(instruction.place, addresses, allocator, inserted) };
    case "store":
      return { ...instruction, place: rewritePlace(instruction.place, addresses, allocator, inserted) };
    case "address":
      return { ...instruction, place: rewritePlace(instruction.place, addresses, allocator, inserted) };
    default:
      return instruction;
  }
}

function rewritePlace(place: MirPlace, addresses: Map<string, AddressEntry>, allocator: (elementType: MirPlace["type"]) => MirValue, inserted: MirInstruction[]): MirPlace {
  switch (place.kind) {
    case "field": {
      if (isIndexedStructPlace(place.base)) {
        const pointer = pointerForIndexedPlace(place.base, addresses, allocator, inserted);
        return { ...place, base: { kind: "deref", pointer, type: place.base.type } };
      }
      return { ...place, base: rewritePlace(place.base, addresses, allocator, inserted) };
    }
    case "index": {
      if (shouldMaterializeIndexedPlace(place)) {
        const pointer = pointerForIndexedPlace(place, addresses, allocator, inserted);
        return { kind: "deref", pointer, type: place.type };
      }
      return { ...place, base: rewritePlace(place.base, addresses, allocator, inserted) };
    }
    case "deref":
    case "param":
    case "local":
      return place;
  }
}

function pointerForIndexedPlace(
  place: Extract<MirPlace, { kind: "index" }>,
  addresses: Map<string, AddressEntry>,
  allocator: (elementType: MirPlace["type"]) => MirValue,
  inserted: MirInstruction[]
): MirValue {
  const key = indexedPlaceKey(place);
  const existing = addresses.get(key);
  if (existing) {
    return existing.pointer;
  }

  const pointer = allocator(place.type);
  inserted.push({ kind: "address", target: pointer, place: clonePlace(place) });
  addresses.set(key, { pointer, dependencies: collectPlaceDependencies(place) });
  return pointer;
}

function isIndexedStructPlace(place: MirPlace): place is Extract<MirPlace, { kind: "index" }> {
  return place.kind === "index" && place.type.kind === "struct";
}

function shouldMaterializeIndexedPlace(place: Extract<MirPlace, { kind: "index" }>): boolean {
  return place.type.kind !== "struct";
}

function indexedPlaceKey(place: Extract<MirPlace, { kind: "index" }>): string {
  return `indexed:${placeKey(place)}`;
}

function createAddressTempAllocator(func: { blocks: Array<{ instructions: MirInstruction[] }> }): (elementType: MirPlace["type"]) => MirValue {
  const used = new Set<string>();
  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      const target = instructionTarget(instruction);
      if (target?.kind === "temp") {
        used.add(target.name);
      }
    }
  }

  let index = 0;
  return (elementType) => {
    while (used.has(`addr${index}`)) {
      index += 1;
    }
    const name = `addr${index}`;
    index += 1;
    used.add(name);
    return { kind: "temp", name, type: { kind: "pointer", elementType } };
  };
}

function invalidateDependency(addresses: Map<string, AddressEntry>, dependency: string): void {
  for (const [key, entry] of addresses) {
    if (entry.dependencies.has(dependency)) {
      addresses.delete(key);
    }
  }
}

function collectPlaceDependencies(place: MirPlace): Set<string> {
  const dependencies = new Set<string>();
  collectPlaceDependencyInto(place, dependencies);
  return dependencies;
}

function collectPlaceDependencyInto(place: MirPlace, dependencies: Set<string>): void {
  switch (place.kind) {
    case "param":
    case "local":
      dependencies.add(dependencyKey(place));
      return;
    case "deref":
      collectValueDependencyInto(place.pointer, dependencies);
      return;
    case "index":
      collectPlaceDependencyInto(place.base, dependencies);
      collectValueDependencyInto(place.index, dependencies);
      return;
    case "field":
      collectPlaceDependencyInto(place.base, dependencies);
      return;
  }
}

function collectValueDependencyInto(value: MirValue, dependencies: Set<string>): void {
  if (value.kind === "local" || value.kind === "param") {
    dependencies.add(dependencyKey(value));
  }
}

function dependencyKey(value: Extract<MirValue, { kind: "local" | "param" }>): string {
  return `${value.kind}:${value.name}`;
}

function instructionTarget(instruction: MirInstruction): MirValue | undefined {
  switch (instruction.kind) {
    case "const_int":
    case "const_bool":
    case "move":
    case "binary":
    case "unary":
    case "compare":
    case "address":
    case "load":
    case "call":
      return instruction.target;
    case "store":
      return undefined;
  }
}

function clonePlace(place: MirPlace): MirPlace {
  switch (place.kind) {
    case "param":
    case "local":
      return { ...place };
    case "deref":
      return { ...place, pointer: cloneValue(place.pointer) };
    case "index":
      return { ...place, base: clonePlace(place.base), index: cloneValue(place.index) };
    case "field":
      return { ...place, base: clonePlace(place.base) };
  }
}

function cloneValue(value: MirValue): MirValue {
  return { ...value } as MirValue;
}

function placeKey(place: MirPlace): string {
  switch (place.kind) {
    case "param":
    case "local":
      return `${place.kind}:${place.name}:${printMirType(place.type)}`;
    case "deref":
      return `deref:${valueKey(place.pointer)}:${printMirType(place.type)}`;
    case "index":
      return `index:${placeKey(place.base)}:${valueKey(place.index)}:${printMirType(place.type)}`;
    case "field":
      return `field:${placeKey(place.base)}:${place.fieldName}:${printMirType(place.type)}`;
  }
}

function valueKey(value: MirValue): string {
  switch (value.kind) {
    case "param":
    case "local":
    case "temp":
      return `${value.kind}:${value.name}:${printMirType(value.type)}`;
    case "const_int":
      return `const_int:${value.text}:${printMirType(value.type)}`;
    case "const_bool":
      return `const_bool:${value.value ? "true" : "false"}`;
  }
}
