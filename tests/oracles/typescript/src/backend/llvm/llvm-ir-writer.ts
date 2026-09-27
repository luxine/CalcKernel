export class LlvmIrWriter {
  private readonly lines: string[] = [];
  private indentLevel = 0;

  line(text = ""): void {
    if (text.length === 0) {
      this.lines.push("");
      return;
    }

    this.lines.push(`${"  ".repeat(this.indentLevel)}${text}`);
  }

  blankLine(): void {
    this.line();
  }

  indent(write: () => void): void {
    this.indentLevel += 1;
    try {
      write();
    } finally {
      this.indentLevel -= 1;
    }
  }

  block(open: string, close: string, write: () => void): void {
    this.line(open);
    this.indent(write);
    this.line(close);
  }

  toString(): string {
    return `${this.lines.join("\n")}\n`;
  }
}
