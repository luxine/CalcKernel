# Naming Conventions

Canonical project naming:

- Language name: IK / IntKernel
- Source file extension: .ik
- Compiler CLI command: ikc

Rules:

- Do not introduce tk, tkc, or .tk aliases.
- Do not rename the language to TK.
- All examples must use .ik.
- All CLI usage examples must use ikc.
- All docs must use IK / IntKernel consistently.
- All tests and snapshots must use ikc and .ik.
- If adding new examples, use examples/*.ik.
- If adding new CLI commands, document them under ikc.
- Do not add compatibility aliases for tkc or .tk unless explicitly requested by the user.

# Documentation Placement

Rules:

- The root `docs/` directory is only for real project documentation that should be shipped, read by users, or maintained as part of IK / IntKernel.
- Do not put AI analysis reports, phase plans, temporary planning notes, readiness reports, or agent working documents under `docs/`.
- Put AI-generated planning and analysis artifacts under `Ai_repository/`.
- If a planning artifact later becomes durable project documentation, rewrite it as user-facing project documentation before moving it into `docs/`.
