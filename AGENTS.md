# Naming Conventions

Canonical project naming:

- Language name: CK / CalcKernel
- Source file extension: .ck
- Compiler CLI command: ckc

Rules:

- Do not introduce tk, tkc, or .tk aliases.
- Do not rename the language to TK.
- All examples must use .ck.
- All CLI usage examples must use ckc.
- All docs must use CK / CalcKernel consistently.
- All tests and snapshots must use ckc and .ck.
- If adding new examples, use examples/*.ck.
- If adding new CLI commands, document them under ckc.
- Do not add compatibility aliases for tkc or .tk unless explicitly requested by the user.

# Documentation Placement

Rules:

- The root `docs/` directory is only for real project documentation that should be shipped, read by users, or maintained as part of CK / CalcKernel.
- Do not put AI analysis reports, phase plans, temporary planning notes, readiness reports, or agent working documents under `docs/`.
- Put AI-generated planning and analysis artifacts under `Ai_repository/`.
- If a planning artifact later becomes durable project documentation, rewrite it as user-facing project documentation before moving it into `docs/`.
