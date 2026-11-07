# FigCraft Android Gemini Design Copilot

Status: implementation design, not yet wired into the application.

## Goal

Add an in-app Gemini copilot to FigCraft on Android. A prompt such as "give this card a blue glow and glass effect" must result in a previewable set of real document edits, not merely a chat response.

## Confirmed integration points

- The Android builder currently auto-generates the FigCraft NativeActivity launcher. A custom adapter is not required just to expose the existing FigCraft engine.
- FigCraft's UI is `figcraft-ui-egui`; its app state is `FigApp` in `crates/ui-egui/src/lib.rs`.
- `FigApp::run(id, params)` in `crates/ui-egui/src/app/commands.rs` routes commands through the same engine used by the UI.
- `figcraft_engine::cmd::all_commands()`, `find_command()`, and `refuse_unknown_params()` expose the command registry and parameter validation.
- Engine document edits are journaled and support undo. The renderer already supports effects including shadows, blur, glass, gradients, and blend modes.

## Intended flow

1. User configures a Gemini API key in Copilot settings.
2. The Android app stores the key using Android Keystore-backed encryption. Never put a key in source, build scripts, resources, or APK constants.
3. The copilot receives a minimal document summary, current selection IDs, and an allow-listed command catalogue. Do not upload the entire document or embedded images by default.
4. Gemini returns a strict JSON action plan. Each action contains a command ID and parameters; free-form executable code is never accepted.
5. Rust validates every command ID and parameter shape against FigCraft's registry. Reject unknown commands, unknown parameters, excessive action counts, invalid target IDs, and unsupported values.
6. Show a human-readable preview. No document mutation occurs before the user taps Apply.
7. Apply validated actions through `FigApp::run` / `Session::execute`, preserving the normal undo history. If an action fails, stop and report the failing step.

## API and privacy requirements

- Use Google's Gemini API, not the Anthropic Messages API.
- Keep API calls off the UI thread and enforce connection/read timeouts.
- Do not log API keys, prompts, document contents, or provider response bodies.
- Show the user what context will be sent. Send only the selected layers and required properties unless the user explicitly expands context.
- Treat model output as untrusted input. Never execute generated shell commands, scripts, Rust, Java, or arbitrary URLs.
- Provide a clear error for missing key, quota/rate limit, network failure, malformed JSON, and rejected commands.

## Implementation sequence

- [ ] Add Android Keystore-backed key storage and a settings entry point.
- [ ] Add a Gemini client with configurable model, timeout, cancellation, and structured response parsing.
- [ ] Add a Rust-side action-plan schema and registry-based validation.
- [ ] Add a compact egui Copilot panel with prompt, response, preview, Apply, and Cancel.
- [ ] Add tests for malformed plans, unknown commands/parameters, invalid IDs, partial failures, and undo.
- [ ] Build the FigCraft Android debug APK and verify on-device that a prompt changes the selected layer and Undo restores it.

## Important builder constraint

The builder reuses the upstream source checkout under `source/figcraft`. Changes made only inside that generated checkout are not durable. The implementation must be kept as a reproducible patch/overlay in this repository or in a maintained FigCraft fork, then applied by the builder before Cargo compilation.


## Basic MCP support

MCP is included in the scope, with a deliberately small first version.

### MVP: expose FigCraft as an MCP server

FigCraft upstream already has an MCP server implementation and a tool catalogue. The Android work should reuse that implementation where it can be embedded safely, rather than inventing a second command system.

Initial tools to expose:
- `inspect_document`: read document metadata and a bounded layer outline.
- `get_node`: read a selected node's supported properties.
- `list_commands`: list the safe, allow-listed design commands and schemas.
- `run_command`: execute one validated FigCraft command through the same engine path as the UI.

Initial transport:
- Prefer MCP Streamable HTTP on loopback for an Android-hosted endpoint if the existing MCP crate can support it without pulling in an unsuitable runtime.
- Bind to localhost by default. Do not expose an unauthenticated MCP endpoint to the LAN.
- If an external MCP client must connect over the network, require an explicit user opt-in and a per-session random bearer token. Stop the server when the user disables it or the app exits.
- Keep stdio as a desktop/CLI transport; Android should not pretend it has a normal desktop stdio lifecycle.

### Safety

- Read-only tools can run without confirmation.
- Every document-mutating tool call requires an explicit in-app approval in the first version.
- Apply the same command/parameter/target validation as the Gemini copilot.
- File access is disabled by default and must remain inside user-selected document locations.
- Enforce request size, execution time, and tool-call limits. Never accept arbitrary shell, script, or filesystem commands.
- Do not log bearer tokens, prompts, document contents, or raw tool payloads.

### Later: MCP client connections

Connecting third-party MCP servers from Android is a separate phase. It requires transport discovery, server trust/permissions, credential storage, and per-tool approval. Do not imply that the first release can connect to arbitrary remote MCP servers.
