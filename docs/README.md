# Remote PTY Router Docs

`init.md` is the original draft. The implementation-facing docs are split as:

- `requirements.md`: product goals, non-goals, UX, and acceptance criteria.
- `product-direction.md`: final unified Fleet + PTY Router product direction.
- `architecture.md`: Rust implementation architecture and command/session model.
- `fleet-capabilities.md`: existing Fleet behavior that rpty must respect.
- `fleet-integration.md`: how Fleet is used, and how output loss is prevented.
- `milestones.md`: staged development plan.
- `validation.md`: local and Fleet validation results.

Current direction: build toward one unified tool. Existing Fleet behavior is the
device-runtime contract; the new PTY router is an additional execution mode for
local Coding Agents. During development the Rust binary may be called `rpty`,
but the final shape should be a Fleet-compatible CLI that includes
`shell`/`agent`/`use` PTY routing.
