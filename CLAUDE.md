# Tauri Desktop App

## Stack
- **Shell**: Tauri 2 (Rust backend + webview frontend)
- **Frontend**: React 18 + TypeScript
- **Styling**: Tailwind CSS v4
- **Icons**: lucide-react
- **Backend Language**: Rust (stable)

## Architecture
```
src/                  # React/TypeScript frontend
  App.tsx             # Root layout + sidebar navigation
  components/         # Reusable UI components
src-tauri/
  src/
    main.rs           # Binary entry point
    lib.rs            # Tauri command wrappers
    core.rs           # Shared business logic
  Cargo.toml          # Rust dependencies
```

## Key Patterns
- Business logic lives in `core.rs`; Tauri commands in `lib.rs` are thin wrappers
- Frontend calls backend with `invoke()` from `@tauri-apps/api/core`
- All `#[tauri::command]` functions must be registered in `generate_handler![]`
- Path safety: validate all user-provided names with `is_valid_name()` before filesystem ops
- Use OS keychain (`keyring` crate) for secrets — never store secrets in plain files

## Commands
```bash
npm run tauri dev    # Full dev mode with hot reload
npm run build        # Frontend type check + bundle
cargo check          # Rust compilation check (from src-tauri/)
cargo test           # Run Rust unit tests
```

<!-- automatic:groups:start -->
## Related Projects
The following projects are related to this one. They are provided for context — explore or reference them when relevant to the current task.

### Aura
The Aurabox application and related projects
**aura**
Location: `../../_active/aura`
**bounce**
Location: `../../_active/bounce`
**lasso**
Location: `../../_active/lasso`
**uhura**
Location: `../../_active/uhura`
**lift**
Location: `../../_active/lift`
**starfleet**
Location: `../../_active/starfleet`
**skills**
Location: `../../_active/skills`
**ravana**
Location: `../ravana`
**tus-server**
Location: `../../_active/tus-server`
**cloud-lib-gcp**
Location: `../../_active/cloud-lib-gcp`
**gcp-pub-sub**
Location: `../../_active/gcp-pub-sub`
**scanfinder**
Location: `../../_active/scanfinder`

### Runbeam
The Runbeam application ecosystem
**harmony-dsl**
Location: `../../../runbeam/runbeam-workspace/projects/harmony-dsl`
**runbeam-workspace**
Location: `../../../runbeam/runbeam-workspace`
**harmony-examples**
Location: `../../../runbeam/runbeam-workspace/projects/harmony-examples`
**harmony-proxy**
Location: `../../../runbeam/runbeam-workspace/projects/harmony-proxy`
**jolt-js**
Location: `../../../runbeam/runbeam-workspace/projects/jolt-js`
**jolt-rs**
Location: `../../../runbeam/runbeam-workspace/projects/jolt-rs`
**runbeam**
Location: `../../../runbeam/runbeam-workspace/projects/runbeam`
**runbeam-cli**
Location: `../../../runbeam/runbeam-workspace/projects/runbeam-cli`
**runbeam-sdk**
Location: `../../../runbeam/runbeam-workspace/projects/runbeam-sdk`
**runbeam-website**: A full-stack SaaS boilerplate with Next.js App Router, Tailwind CSS, Prisma ORM, and NextAuth.js. Includes authentication flows, dashboard layout, billing hooks, and a component library ready for rapid product development.
Location: `../../../runbeam/runbeam-workspace/projects/website`
**docs**
Location: `../../../runbeam/runbeam-workspace/projects/docs`

<!-- automatic:groups:end -->
