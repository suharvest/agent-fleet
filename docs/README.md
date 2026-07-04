# AgentFleet Docs

`init.md` is the original draft. The implementation-facing docs are split as:

- `requirements.md`: product goals, non-goals, UX, and acceptance criteria.
- `product-direction.md`: final AgentFleet product direction.
- `architecture.md`: Rust implementation architecture and command/session model.
- `fleet-capabilities.md`: existing Fleet behavior that rpty must respect.
- `fleet-integration.md`: how Fleet is used, and how output loss is prevented.
- `milestones.md`: staged development plan.
- `validation.md`: local and Fleet validation results.

Current direction: AgentFleet is one unified tool. Existing Fleet behavior is
the device-runtime contract; the PTY router is an additional execution mode for
local Coding Agents. The Rust package exposes `fleet` as the stable user-facing
CLI and keeps `rpty` as a compatibility/development entry point.
