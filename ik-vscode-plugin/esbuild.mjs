import esbuild from "esbuild";

await esbuild.build({
  entryPoints: ["src/extension.ts"],
  bundle: true,
  platform: "node",
  target: "node20",
  format: "cjs",
  outfile: "dist/extension.cjs",
  external: ["vscode"],
  sourcemap: true,
  logLevel: "info"
});
