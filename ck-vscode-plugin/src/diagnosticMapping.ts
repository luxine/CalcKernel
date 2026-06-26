export interface SourcePositionLike {
  offset: number;
  line: number;
  column: number;
}

export interface SourceSpanLike {
  start: SourcePositionLike;
  end: SourcePositionLike;
}

export interface RangePositionCoordinates {
  line: number;
  character: number;
}

export interface RangeCoordinates {
  start: RangePositionCoordinates;
  end: RangePositionCoordinates;
}

export function spanToRangeCoordinates(text: string, span: SourceSpanLike): RangeCoordinates {
  const lines = splitLines(text);
  const start = clampPosition(lines, span.start.line - 1, span.start.column - 1);
  let end = clampPosition(lines, span.end.line - 1, span.end.column - 1);

  if (comparePositions(end, start) < 0) {
    end = { ...start };
  }

  if (samePosition(start, end)) {
    const lineLength = lines[start.line]?.length ?? 0;
    if (start.character < lineLength) {
      end.character = start.character + 1;
    } else if (start.character > 0) {
      start.character -= 1;
    }
  }

  return { start, end };
}

function splitLines(text: string): string[] {
  const lines = text.split(/\r\n|\r|\n/);
  return lines.length > 0 ? lines : [""];
}

function clampPosition(lines: string[], line: number, character: number): RangePositionCoordinates {
  const clampedLine = clamp(line, 0, Math.max(0, lines.length - 1));
  const lineLength = lines[clampedLine]?.length ?? 0;
  return { line: clampedLine, character: clamp(character, 0, lineLength) };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function comparePositions(left: RangePositionCoordinates, right: RangePositionCoordinates): number {
  if (left.line !== right.line) {
    return left.line - right.line;
  }
  return left.character - right.character;
}

function samePosition(left: RangePositionCoordinates, right: RangePositionCoordinates): boolean {
  return left.line === right.line && left.character === right.character;
}
