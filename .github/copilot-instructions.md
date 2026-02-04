# Copilot Instructions for ritsu-sonnet

This repository currently contains only Git metadata. These instructions are written to guide future Copilot CLI sessions once code and configuration are added.

---

## Build, test, and lint commands
- No build/test/lint commands are present yet in the repository. When added, document the following in this file:
  - Full test suite: e.g. `npm test` or `pytest`
  - Single-test command: e.g. `npm test -- -t "TestName"` or `pytest path/to/test::TestClass::test_method`
  - Lint command: e.g. `npm run lint` or `flake8 .`
  - Build command: e.g. `npm run build` or `./gradlew assemble`

Include any environment variables or setup steps required before running these commands.

## High-level architecture
- Repository currently has no source files. When the project is added, include here:
  - Overall purpose of the project (service, library, CLI, web app)
  - Major components (backend API, frontend app, worker, DB schema, infra templates) and where they live (paths / packages)
  - How components interact (network calls, event bus, shared libraries)
  - Entry points (CLI commands, web server start file, main package)

Keep this section to the minimal "big picture" notes that require reading multiple files to understand.

## Key conventions
- If present, document repository-specific conventions such as:
  - Branching and release naming conventions (e.g., `main`, `release/*`)
  - Where to find configuration (e.g., `config/`, `.env.example`) and how it's loaded
  - Directory layout conventions (e.g., `src/` vs `lib/`, `packages/` for monorepos)
  - Test organization (unit vs integration folders), and how to run a single test
  - Any code generation steps or build-time codegen tools and where generated code is checked in

## Existing AI assistant configs to incorporate
If any of the following files exist, copy important project-specific rules to this file:
- CLAUDE.md
- AGENTS.md
- .cursorrules or .cursor/rules/
- .windsurfrules
- CONVENTIONS.md or AIDER_CONVENTIONS.md
- .clinerules or .cline_rules

(There are no such files in the repository currently.)

## How Copilot CLI should operate here
- First step: list repo files and look for README/CONTRIBUTING/CI configs.
- If repository appears empty (only .git), ask the user whether to initialize files or point to the correct path.
- Prefer grep/glob for searching and avoid running broad builds until minimal CI/dev dependencies are available.
- When making changes, keep edits minimal and run existing tests/lints when possible.

---

If you’d like, update this file once the project code and CI are present so Copilot sessions can provide more precise automation and shortcuts.
