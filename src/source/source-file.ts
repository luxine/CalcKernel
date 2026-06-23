export interface SourcePosition {
  offset: number;
  line: number;
  column: number;
}

export interface SourceSpan {
  start: SourcePosition;
  end: SourcePosition;
}

export class SourceFile {
  constructor(
    readonly fileName: string,
    readonly text: string
  ) {}
}
