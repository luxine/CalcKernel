# Publishing the CalcKernel VS Code Extension

This document describes how to package and publish `calckernel-vscode-plugin` so other users can install it from the Visual Studio Marketplace.

Official references:

- VS Code Publishing Extensions: https://code.visualstudio.com/api/working-with-extensions/publishing-extension
- VS Code Continuous Integration: https://code.visualstudio.com/api/working-with-extensions/continuous-integration
- Marketplace publisher management: https://marketplace.visualstudio.com/manage/publishers/

## Current Package Status

The extension package is already buildable with `vsce`:

```sh
cd <repo>/ik-vscode-plugin
pnpm install
pnpm test
pnpm compile
pnpm package
```

The current manifest uses:

```json
{
  "name": "calckernel-vscode-plugin",
  "displayName": "CalcKernel",
  "publisher": "luxine",
  "version": "0.1.0"
}
```

The current Marketplace publisher target is `luxine`. The public extension identifier will be `luxine.ck-vscode-plugin`.

## One-Time Marketplace Setup

1. Sign in with a Microsoft account.
2. Create or choose an Azure DevOps organization.
3. Create a Visual Studio Marketplace publisher at:

   https://marketplace.visualstudio.com/manage/publishers/

4. Choose a stable publisher ID. This ID cannot be changed after creation.
5. Confirm `package.json` uses the Marketplace publisher ID:

   ```json
   {
     "publisher": "luxine"
   }
   ```

6. Authenticate `vsce`.

   For local manual publishing, the simplest setup is a Personal Access Token (PAT) with Marketplace `Manage` scope:

   ```sh
   pnpm exec vsce login <publisher-id>
   ```

   Paste the PAT when prompted.

Microsoft's current VS Code documentation says global Azure DevOps PATs retire on December 1, 2026. For long-term automated publishing, prefer Microsoft Entra ID based publishing instead of a long-lived PAT.

## Pre-Publish Checklist

Run these checks before every public release:

```sh
cd <repo>/ik-vscode-plugin
pnpm test
pnpm compile
pnpm package
code --install-extension calckernel-vscode-plugin-0.1.0.vsix --force
```

Then verify in VS Code:

- `.ck` files open as `CalcKernel`.
- Syntax highlighting and semantic highlighting are visible.
- Hover shows useful type/signature information.
- Completions work for keywords, local symbols, and struct fields.
- `configs[0].` offers field completions.
- Go to Definition works for declarations and references.
- The Outline view lists structs, functions, fields, parameters, and locals.
- Compiler diagnostics appear in the Problems panel.

Recommended Marketplace polish before the first public release:

- Add a public `repository` field to `package.json`.
- Keep `LICENSE` in the extension root, or make sure the repository license is clear.
- Add a 128x128 PNG icon and reference it with `package.json` `icon`.
- Add screenshots or GIFs to the README using HTTPS image URLs.
- Confirm `.vscodeignore` excludes source-only files that are not needed at runtime.

## Publish from the Command Line

After `publisher` is set and `vsce login` succeeds:

```sh
cd <repo>/ik-vscode-plugin
pnpm test
pnpm compile
pnpm exec vsce publish --no-dependencies
```

To increment the extension version and publish in one command:

```sh
pnpm exec vsce publish patch --no-dependencies
pnpm exec vsce publish minor --no-dependencies
pnpm exec vsce publish major --no-dependencies
```

Be careful with `vsce publish patch|minor|major`: when run inside a git repository, it can update `package.json` and create a version commit/tag.

## Publish by Manual Upload

If you do not want the CLI to publish directly:

```sh
cd <repo>/ik-vscode-plugin
pnpm package
```

Then upload the generated `.vsix` file from the Marketplace publisher management page:

https://marketplace.visualstudio.com/manage/publishers/

This is the safest first-release path because you can inspect the packaged VSIX before uploading it.

## GitHub Actions Publishing

For CI-based publishing:

1. Store the publishing token as a GitHub Actions secret named `VSCE_PAT`.
2. Add a deploy script without hardcoding the token:

   ```json
   {
     "scripts": {
       "deploy": "vsce publish --no-dependencies"
     }
   }
   ```

3. Run tests and packaging before publishing.
4. Only publish on release tags, for example `v0.1.0`.

Minimal publishing step:

```yaml
- name: Publish VS Code extension
  if: success() && startsWith(github.ref, 'refs/tags/')
  run: pnpm run deploy
  env:
    VSCE_PAT: ${{ secrets.VSCE_PAT }}
```

For long-term CI after December 1, 2026, use Microsoft Entra ID based publishing instead of relying on global Azure DevOps PATs.

## After Publishing

After the extension appears on Marketplace:

1. Install it from the VS Code Extensions view by searching `CalcKernel`.
2. Confirm the public extension ID is `luxine.ck-vscode-plugin`.
3. Check the Marketplace page renders README, changelog, license, icon, and screenshots correctly.
4. Keep release notes in `CHANGELOG.md` aligned with `package.json` `version`.
5. Use a new semver version for every update. Marketplace versions cannot be reused after publication.
