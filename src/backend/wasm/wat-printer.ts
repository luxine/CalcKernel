export class WatPrinter {
  private readonly lines: string[] = [];
  private indentLevel = 0;

  line(text = ""): void {
    if (text.length === 0) {
      this.lines.push("");
      return;
    }

    this.lines.push(`${"  ".repeat(this.indentLevel)}${text}`);
  }

  open(text: string): void {
    this.line(text);
    this.indentLevel += 1;
  }

  close(text = ")"): void {
    this.indentLevel -= 1;
    this.line(text);
  }

  print(): string {
    return `${this.lines.join("\n")}\n`;
  }
}
