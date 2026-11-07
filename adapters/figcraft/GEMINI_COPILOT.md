# FigCraft Android Gemini Design Copilot

Status: partial implementation. The bounded Android UI-thread bridge, embedded upstream MCP server, opt-in loopback HTTP transport, and an in-app approval prompt for `engine.execute` are committed. Gemini, the MCP settings/credential screen, compilation, and APK verification remain outstanding.

## Goal

Add an in-app Gemini copilot to FigCraft on Android. A prompt such as "give this card a blue glow and glass effect" must result in a previewable set of real document edits, not merely a chat response.

## Confirmed integration points

- The Android builder currently auto-generates the FigCraft NativeActivity launcher. The generator (version 73) copies `templates/android/figcraft_control_bridge.rs`, adds the `figcraft-mcp` dependency, and injects the UI-thread drain hook.
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
- [ ] Build the FigCraft Android debug APK and verify on-device that read-only MCP requests inspect the visible document.

## Important builder constraint

The builder reuses the upstream source checkout under `source/figcraft`. Changes made only inside that generated checkout are not durable. The implementation must be kept as a reproducible patch/overlay in this repository or in a maintained FigCraft fork, then applied by the builder before Cargo compilation.


## Basic MCP support

MCP is included in the scope, with a deliberately small first version.

### MVP: expose FigCraft as an MCP server

FigCraft upstream already has an MCP server implementation and a tool catalogue. The Android work should reuse that implementation where it can be embedded safely, rather than inventing a second command system.

The upstream tool catalogue is reused. The Android backend allows read-only control methods (`document.inspect`, `node.get`, `engine.commands`, `ui.inspect`, `ui.render`, `ui.screenshot`, and `window.list`) plus the read-only `comment.list` engine command. Other `engine.execute` calls are queued behind an in-app approval sheet that displays the command and parameters. File operations, pointer/key simulation, and event polling remain denied.

Transport implemented (initial slice):
- MCP Streamable HTTP on `127.0.0.1:38761/mcp`: authenticated `POST` JSON requests, a server-issued `Mcp-Session-Id`, required protocol-version/session headers after initialization, `DELETE` to end the session, and bounded 32 MiB request bodies. It never binds to the LAN.
- The endpoint is disabled by default. For developer testing only, create `mcp-server.enabled` in the directory returned by `AndroidApp::internal_data_path()` before launching. The app creates `mcp-server.token` beside it with a random 256-bit token. The token is never logged.
- With a debuggable APK and the default package name, the developer flow is: `adb shell run-as com.artcraft.figcraft touch files/mcp-server.enabled`, restart the app, read the token using `adb shell run-as com.artcraft.figcraft cat files/mcp-server.token`, then forward the port with `adb forward tcp:38761 tcp:38761`. Replace the package name if the build uses `-packagename=...`.
- This first transport slice does not implement server-sent events or multiple simultaneous MCP sessions; one client session is active at a time, and the client can send DELETE to release it. GET is rejected because this server does not open an SSE stream. An in-app enable/disable/settings screen is still missing. Remove the enable marker and restart the app to disable the server.
- The endpoint rejects every HTTP `Origin` header. It is intended for native/local MCP clients only, not browser-based clients; this avoids accepting browser-origin requests against a loopback service. It also requires bearer authentication and binds only to `127.0.0.1`.
- The embedded upstream MCP crate currently advertises protocol versions `2025-06-18`, `2025-03-26`, and `2024-11-05`. It does not implement the newer `2026-07-28` stateless protocol revision.
- Keep stdio as a desktop/CLI transport; Android does not pretend it has a normal desktop stdio lifecycle.

### Safety

- Read-only tools can run without confirmation.
- Every document-mutating tool call requires an explicit in-app approval in the first version.
- Apply the same command/parameter/target validation as the Gemini copilot.
- File access is disabled by default and must remain inside user-selected document locations.
- Enforce request size, execution time, and tool-call limits. Never accept arbitrary shell, script, or filesystem commands.
- Do not log bearer tokens, prompts, document contents, or raw tool payloads.

### Later: MCP client connections

Connecting third-party MCP servers from Android is a separate phase. It requires transport discovery, server trust/permissions, credential storage, and per-tool approval. Do not imply that the first release can connect to arbitrary remote MCP servers.


### Android-specific feasibility finding

The upstream implementation is reusable at the protocol/tool layer, but it is **not yet an Android-ready server**:

- `figcraft-mcp` currently implements newline-delimited JSON-RPC over stdio. Its public `Server::serve` accepts generic buffered input/output, and `Server::handle_line` handles one JSON-RPC message.
- `figcraft-mcp::Headless` owns a separate `Session`; using it would edit a detached document, not the document currently open in the Android canvas. It is not an acceptable integration for this feature.
- The upstream `Attached` backend speaks FigCraft's localhost control protocol. The desktop app starts that control server only when launched with `--control`; the generated Android launcher currently does not start it.
- The Android launcher creates a live `FigApp` instance. MCP requests must therefore be marshalled onto the UI/app thread and applied to that same session. A background socket thread must never mutate `FigApp` or its `Session` directly.

### Revised implementation plan

1. Implemented: bounded request queue drained into `FigApp.control_queue` on the UI thread.
2. Implemented: adapter for upstream `figcraft_mcp::Server` dispatches read-only backend calls to the live Android `FigApp`; it does not use `Headless`.
3. Implemented: loopback-only MCP Streamable HTTP POST/JSON transport with bearer token, server-issued session ID, DELETE termination, and bounded body size. GET/SSE and multiple concurrent sessions are not implemented yet.
4. Implemented as a developer-only gate: the server starts only when `files/mcp-server.enabled` exists in app-private storage. A UI toggle and credential display are still needed.
5. Implemented: `engine.execute` displays an in-app approval sheet and only reaches FigCraft after explicit approval. `comment.list` is the only command exception because it is read-only; file operations, pointer/key simulation, and event polling are denied.
6. Pending: run compile/tests and add end-to-end protocol tests on a real device.

This keeps the MCP feature grounded in the actual FigCraft session and avoids the misleading shortcut of launching a second, invisible document.


### Confirmed Android UI-thread bridge hook

A closer read of the current builder generator and FigCraft UI lifecycle found a clean integration seam:

- The builder generates an `AndroidShell` wrapper around `FigApp`; for FigCraft, the wrapper's UI callback calls `self.inner.ui(ui)`.
- `FigApp::ui` itself calls `control::drain(self, &ctx)`, which processes `FigApp.control_queue` on the UI thread.
- Therefore the Android wrapper can safely extend `self.inner.control_queue` from a bounded, thread-safe queue immediately before calling `self.inner.ui(ui)`. This avoids modifying FigCraft's document engine and preserves its existing command validation, undo, and screenshot behavior.
- The desktop app's `apps/figcraft/src/control_server.rs` already implements the bounded localhost JSON-lines transport and request timeout/cancellation gates. It can serve as the reference for a builder-owned Android control bridge, but it cannot simply be imported because the Android launcher is a separate crate.

The remaining implementation work is now sharply scoped:

1. Add an in-app enable/disable control and display the endpoint credentials only inside the app. The current file-marker gate is developer-only.
2. Add GET/SSE support and independent MCP session handling if required by target clients.
3. Implemented: the generated Android wrapper draws an approval window after the live FigCraft canvas; `engine.execute` is dispatched only after the user taps Approve. All other mutating control methods remain blocked.
4. Add Gemini Keystore storage, structured action planning, preview, apply, and undo flow.
5. Verify the Android build and exercise initialize, tools/list, read-only tools/call, approval-gated mutation, and Undo on a real device.

This is a more direct route than adding a separate backend to the engine: the existing `FigApp::ui → control::drain → Session` path already does the important UI-thread and history work.
