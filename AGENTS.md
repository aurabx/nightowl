# NightOwl Project Instructions

## Project Overview

NightOwl is a desktop application built with Tauri 2, combining a Rust backend with a React + TypeScript frontend. The application appears to be a DICOM service tester/management tool with peer connectivity, worklist management, and MCP (Model Context Protocol) integration capabilities.

**Primary Tech Stack:**
- **Desktop Shell**: Tauri 2.11 (Rust + webview)
- **Frontend**: React 19, TypeScript 6, Vite 7
- **Styling**: Tailwind CSS v4 (via `@tailwindcss/vite`)
- **Icons**: lucide-react
- **Backend**: Rust (stable toolchain)
- **Build Tool**: Vite for frontend, Cargo for Rust

## Build & Run Commands

**Development:**
- `make dev` or `npm run tauri dev` — Launch full dev mode with hot reload
- `make web` or `npm run dev` — Run frontend dev server only (no Tauri shell)

**Building:**
- `make build` or `npm run tauri build` — Build release bundle (desktop app)
- `make build-web` or `npm run build` — Type-check and bundle frontend only (`tsc -b && vite build`)

**Quality Gates:**
- `make check` — Run both Rust and TypeScript compile checks
- `make test` — Run Rust unit and doc tests (`cargo test`)
- `make lint` — Run `cargo clippy` with warnings as errors
- `make fmt` — Auto-format Rust code; `make fmt-check` to verify without rewriting

**Utilities:**
- `make install` — Install npm dependencies
- `make icons ICON_SRC=path/to/source.png` — Regenerate Tauri icon set from a 1024×1024 source
- `make clean` — Remove build artifacts (target/, dist/)
- `make kill-dev` — Force-kill lingering dev processes and free ports (5173, 11112, 11113)

## Architecture Overview

**Frontend (src/):**
- `App.tsx` — Root layout with sidebar navigation
- `components/` — Reusable UI components (Field, Modal, Pagination, Select, Sidebar)
- `pages/` — Route pages (About, Activity, Mcp, Peers, Scu, Settings, Store, Worklist)
- `lib/api.ts` — Frontend API layer for backend communication
- `main.tsx` — React entry point

**Backend (src-tauri/src/):**
- `main.rs` — Binary entry point, Tauri app initialization
- `lib.rs` — Tauri command wrappers (thin layer calling `core.rs`)
- `core.rs` — Shared business logic (all substantive backend code lives here)
- `core/` — Additional core modules (directory present but contents not shown)

**Configuration:**
- `tauri.conf.json` — Tauri app configuration (identifier, permissions, build settings)
- `capabilities/default.json` — Tauri capability definitions
- `Cargo.toml` — Rust dependencies and metadata
- `package.json` — Node dependencies and scripts
- `vite.config.ts`, `tsconfig.json` — Frontend build configuration

## Coding Conventions

**Rust (Backend):**
- Business logic must live in `core.rs` or `core/*` modules; `lib.rs` functions are thin wrappers only
- All `#[tauri::command]` functions must be registered in `generate_handler![]` in `main.rs`
- **Path Safety**: Validate all user-provided filesystem names with `is_valid_name()` before any file operations
- **Secrets**: Use OS keychain (`keyring` crate) — never store secrets in plaintext files
- Follow standard Rust formatting (`cargo fmt`); pass `cargo clippy` with no warnings

**TypeScript (Frontend):**
- Backend calls use `invoke()` from `@tauri-apps/api/core`
- Strict TypeScript enabled; no implicit `any`
- React 19 patterns (functional components, hooks)
- Tailwind CSS v4 for styling (utility classes)

**Naming & Style:**
- Rust: snake_case for functions/variables, PascalCase for types
- TypeScript: camelCase for functions/variables, PascalCase for components/types
- Components use `.tsx` extension; modules use `.ts`
- Keep components focused and reusable

**Error Handling:**
- Tauri commands return `Result<T, String>` (or appropriate error type)
- Frontend should handle errors gracefully with user feedback
- Log errors appropriately for debugging

## Agent Guidance

**DO:**
- Always run `make check` before committing to catch type and compilation errors
- Run `make test` to verify Rust unit tests pass
- Use `make fmt` to format Rust code before committing
- Consult PLAN.md and PLAN-NEXT.md for context on planned features and current priorities
- Register new Tauri commands in both `lib.rs` (with `#[tauri::command]`) and `main.rs` (`generate_handler![]`)
- Validate user input, especially filesystem paths, before processing
- Ask before making significant architectural changes
- Use the Automatic MCP service: call `automatic_search_memories` at session start for project context; `automatic_store_memory` at session end to capture learnings

**DO NOT:**
- Commit secrets, API keys, or credentials (use OS keychain instead)
- Delete files without confirmation, especially in `src-tauri/icons/` or configuration files
- Modify generated files in `src-tauri/gen/` directly
- Skip type-checking or tests before committing
- Place business logic in `lib.rs` (it belongs in `core.rs`)
- Hardcode port numbers (use existing patterns: 5173 for Vite, 11112/11113 for DICOM services)

**When Stuck:**
- Check existing code patterns in similar pages/components
- Review Makefile for available commands and smoke tests
- Use `make help` to see all documented targets
- Search memories (`automatic_search_memories`) for past decisions or solutions
- Refer to related Aura projects (listed in AGENTS.md/CLAUDE.md) for architectural patterns

**Automatic MCP Integration:**
- At session start: call `automatic_list_skills`, `automatic_search_memories`, and `automatic_read_project`
- During work: use `automatic_search_skills` for domain-specific guidance
- At session end: call `automatic_store_memory` with meaningful learnings (decisions, conventions, gotchas)
- Use hierarchical memory keys (e.g., `conventions/naming`, `setup/dicom`, `decisions/mcp-integration`)

<!-- automatic:rules:start -->
# Working with the Automatic MCP Service

This project is managed by Automatic, a desktop hub that provides skills, rules, hooks, memory, feature tracking, and MCP server configs to agents via an MCP interface. The Automatic MCP server is always available in this project.

## Session Start

1. Call `automatic_list_skills` to discover available skills. If any match the current task domain, call `automatic_read_skill` to load instructions and companion resources.
2. Call `automatic_search_memories` with relevant keywords for this project to retrieve past learnings, conventions, and decisions.
3. Call `automatic_read_project` with this project's name to understand the configured skills, MCP servers, agents, and directory.

## During Work

- **Skills** — Follow loaded skill instructions. Skills may include companion scripts, templates, or reference docs in their directory.
- **MCP Servers** — Call `automatic_list_mcp_servers` to see what servers are registered. Call `automatic_sync_project` after configuration changes.
- **Skill Discovery** — Call `automatic_search_skills` to find community skills on skills.sh when you need specialised guidance not covered by installed skills.
- **Related Projects** — Before searching the filesystem or asking the user for sibling projects, call `automatic_get_related_projects` with this project's name. It returns peer projects (name, description, directory, and the relative path from this project) for every Project Group this project belongs to. This is the authoritative source — related projects are intentionally not written into the instruction file.
- **Other Projects** — Call `automatic_list_projects` to see every project name registered in Automatic.
- **Registering Projects** — Call `automatic_register_project` with a unique name and an absolute directory path to bring a new project under Automatic management. Optionally pass agent ids (e.g. `claude`) to sync their config files immediately. The call is refused when the directory already belongs to a registered project or holds an unregistered Automatic config — ask the user how to proceed in those cases.
- **Project Context** — Call `automatic_get_project_context` for a project's commands, entry points, architecture concepts, conventions, gotchas, a merged documentation index, and the rules currently attached to each instruction file.

## Rules

Rules are markdown instruction blocks attached to a project's instruction files (this file is one of them):

- `automatic_list_rules` — list every rule in the library (machine name, display name, plugin owner if any).
- `automatic_read_rule` — read a rule's full content by machine name.
- `automatic_create_rule` / `automatic_update_rule` — add a new rule or edit an existing one's name and/or content. `automatic_update_rule` refuses plugin-provided rules.
- `automatic_attach_rule` / `automatic_detach_rule` — wire a rule into a project's instruction file. Neither call syncs to disk on its own — call `automatic_sync_project` afterwards.
- `automatic_delete_rule` — remove a rule from the library. Mandatory rules (including this one) and plugin-provided rules cannot be deleted. Deleting a rule does not detach it from projects that reference it; they silently skip it on next sync.

## Hooks

Hooks are event-triggered handlers (e.g. on session start, before a tool call) scoped to a specific agent and event:

- `automatic_list_hooks` — list every hook in the library (machine name, name, agent, event, plugin owner if any).
- `automatic_read_hook` — read a hook's full definition (name, agent, event, matcher, handler, timeout).
- `automatic_create_hook` / `automatic_update_hook` — add a new hook or edit an existing one.
- `automatic_delete_hook` — remove a hook from the library. Plugin-provided hooks cannot be deleted. Projects referencing a deleted hook silently skip it on next sync.
- `automatic_attach_hook` / `automatic_detach_hook` — wire a hook into a project (the target agent is inferred from the hook's library record). Neither call syncs to disk on its own — call `automatic_sync_project` afterwards.

## Memory

Use the memory tools to persist and retrieve project-specific context across sessions:

- `automatic_store_memory` — store a key-value entry. Set the `source` parameter so the origin is traceable. Use descriptive, hierarchical keys (e.g. `conventions/naming`, `setup/database`, `decisions/auth-approach`).
- `automatic_get_memory` — retrieve a specific entry by key.
- `automatic_list_memories` — list every stored entry, optionally filtered by a key pattern.
- `automatic_search_memories` — case-insensitive substring search across keys and values. Search before making assumptions; previous sessions may have captured relevant context.
- `automatic_delete_memory` — remove a single entry by key.
- `automatic_clear_memories` — remove all entries for a project, optionally filtered by pattern. Requires explicit confirmation and cannot be undone; use with caution.
- `automatic_read_claude_memory` — read Claude Code's own auto-memory files for this project (`MEMORY.md` and any topic files under `~/.claude/projects/<encoded-path>/memory/`). Use this to see what Claude has already learned, then call `automatic_store_memory` to promote anything durable into Automatic's structured store.

## Features

Automatic provides project-scoped feature tracking for managing work items across sessions:

- Call `automatic_list_features` to see planned work. Filter by state (`backlog`, `todo`, `in_progress`, `review`, `complete`, `cancelled`). Pass `include_archived: true` to list archived features instead.
- Before starting a task, call `automatic_set_feature_state` to move it to `in_progress`.
- During work, call `automatic_add_feature_update` to log significant progress, decisions, or blockers. Updates are append-only and ordered newest-first.
- On completion, move the feature to `review` so the user can verify before marking `complete`.
- If new work is discovered, call `automatic_create_feature` to capture it in the backlog.
- Use `automatic_get_feature` for full detail on one feature, `automatic_update_feature` to edit its metadata (title, description, priority, assignee, tags, linked files, effort), and `automatic_archive_feature` / `automatic_unarchive_feature` to hide or restore one without losing its state. `automatic_delete_feature` permanently removes a feature and all its updates; this cannot be undone.

## Credentials

Call `automatic_get_credential` to retrieve a stored API key for a known LLM provider (e.g. `anthropic`, `openai`). Only recognised provider ids are accepted.

## Sessions

Call `automatic_list_sessions` to see active Claude Code sessions tracked by Automatic's hooks (session id, working directory, model, started_at).

## Session End

Before finishing a session, call `automatic_store_memory` to capture any new project-specific rules, pitfalls, setup steps, or decisions discovered during the session. This prevents knowledge loss across sessions.

# Agent Problem-Solving Process

A framework for structured, honest, and traceable software development work. Apply judgement at each stage. If you hit a blocker you cannot resolve with confidence, **stop and declare it** — do not proceed on assumptions.

USE OF THIS FRAMEWORK IS NON-NEGOTIABLE. Acknowledge that you have read this file before starting.

---

## Phase 1: Understand the Task

- Restate the goal in your own words. Confirm what problem is being solved, not just what action is requested.
- Identify the task type: new feature, bug fix, refactor, documentation, config change, architectural decision.
- Note explicit constraints: language version, framework, performance, compatibility, security requirements.
- Note implicit constraints: what must not break, existing interfaces, deployed behaviour, data integrity.
- If the task is ambiguous or contradictory, **ask before proceeding**. Assumptions made here compound through every later phase.

## Phase 2: Understand the Context

- Read the relevant files. Do not rely on filenames or structure alone.
- Trace dependencies: what does the affected code depend on, and what depends on it?
- Check how similar problems have been solved elsewhere in the codebase. Prefer consistency.
- Identify existing test coverage. Understand what is already verified and what is not.
- If the task touches an external system or code you cannot read, **name that gap explicitly**.
- **Reusable commands.** When this project has repo-local commands, check `.agents/commands-index.md` before starting work that may match a reusable workflow. If the index lists a relevant command, read the referenced file in `.agents/commands/` and follow it. Treat these files as reusable workflow instructions, not as native slash commands.

## Phase 3: Plan

- Outline your approach before writing any code. It does not need to be exhaustive — it needs to be honest.
- Prefer the minimal scope of change that correctly solves the problem. Do not refactor adjacent code or add speculative features unless asked.
- Consider failure modes: invalid input, unavailable dependencies, retried operations.
- Validate your plan against the constraints from Phase 1. If there is a conflict, surface it rather than quietly working around it.

## Phase 4: Communicate

- Tell the user what you found, what needs to be done, and how you are going to fix it.
- Communicate in plain, clear language. Do not use jargon, idioms, turns-of-phrase or colloquialisms.
- Communicate in full sentances, do not omit words or drop articles.
- Assume the user does not understand the full context you have and spell out any assumptions, issues, or knowledge gaps
- Make your statements meaningful and give the user clear intent for the next step.

## STOP

At this point, you need permission to continue.

## Before the first Write, Edit, NotebookEdit, or Bash call in a task that changes
a file or runs a state-changing command — stop.

State the plan in full as the entire reply. End the turn there — no tool call
in the same message. Wait for a reply before the first mutating call.
A ticket, backlog item, task assignment, or "work on X" is not that reply,
even if it says "proceed" or "update status as you progress." The reply has
to respond to the specific plan just stated, not to the existence of the task.

Read-only calls (Read, Grep, ToolSearch, and similar) are exempt — explore
freely before the plan.

Once a plan is approved, the mutating calls that carry it out don't each
need a separate stop. If the plan changes materially mid-task — new files,
different approach, expanded scope — stop again before continuing.

**Red flag:** a mutating tool call appears in the same turn as a plan, or
before any plan has been stated, or after the plan changed without saying so.

## Phase 5: Implement

- Edit only what is relevant to the task. If you notice a bug nearby, note it — do not silently fix it unless it is in scope.
- Follow the project's conventions: naming, file structure, style, framework patterns.
- Write type-safe, deterministic, defensively validated code. Refer to the project's coding patterns document.
- Leave no placeholders or stubs without declaring them. Incomplete work must be disclosed, not hidden.
- Comment on *why*, not *what*. Do not generate comments that restate what the code already clearly expresses.
- Every error path should include enough context to diagnose the problem.

## Phase 6: Verify

- Review your changes as if reading someone else's code. Check for logic errors, edge cases, and missing error handling.
- Confirm the implementation actually solves the goal from Phase 1. Trace through it with a realistic input.
- Consider what existing behaviour may have been affected. Run tests if they exist; note the gap if they do not.
- Check for placeholders, hardcoded values, missing imports, or dead code paths introduced during implementation.

## Phase 7: Summarise

- Summarise what you did and why, including significant decisions.
- Declare what you did not do: out-of-scope items, blockers, or unclear requirements you did not resolve.
- Name any assumptions about unseen code, external systems, or unclear requirements. Do not present uncertain work as definitive.
- Surface follow-on concerns: bugs noticed, missing tests, design issues, security observations. Do not discard observations silently.
- Do not exaggerate confidence. If you are uncertain, say so.

# Automatic-managed .gitignore

This project ignores the agent configuration that Automatic generates.

Automatic writes the instruction files and agent config directories in this
project. It also keeps a managed block in `.gitignore` that lists those paths.
The block is bounded by these markers:

```
# BEGIN Automatic-managed
...
# END Automatic-managed
```

Follow these rules:

1. Do not commit the ignored files. They are generated. Automatic rewrites them
   on every sync, so committing them causes churn and merge conflicts.
2. Do not edit inside the managed block. Automatic regenerates it on each sync.
   Any manual change between the markers is lost.
3. Do not remove the managed block to force these files into version control. If
   the team wants to share agent config through git, turn off "Manage .gitignore"
   for this project in Automatic instead. That removes the block cleanly.
4. Add your own ignore entries outside the markers. Automatic never touches the
   rest of the file.
<!-- automatic:rules:end -->
