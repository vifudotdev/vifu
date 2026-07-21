# OpenAI Build Week 2026 Development Record

This document collects the submission scope, implementation evidence, Codex
collaboration, verification results, and known limitations.

The implementation and local verification are finalized on the `dev` branch.
The submission tag points to this record together with the reviewed source.

## Submission Scope

| Item | Value |
| --- | --- |
| Track | Developer Tools |
| Baseline commit | [`f5f183c`](https://github.com/vifudotdev/vifu/commit/f5f183c960aa66af15f3ddbce56b3f66fa3a7687) |
| Final reviewed source commit | [`a40f455`](https://github.com/vifudotdev/vifu/commit/a40f455) |
| Submission branch | `dev` |
| Submission tag | `build-week-2026` |
| Primary Codex session | `019f4b00-f1fc-72b2-8028-bdd3e2875381` |

Only committed changes after the baseline are included as submission work.

## Submitted Work

The following work was committed after the baseline and is currently in the
submission scope:

| Work | Evidence |
| --- | --- |
| Extracted a shared Rust core for configuration, protocol, relay, and session behavior | `969b8fd` |
| Added a typed Agent Gateway protocol package | `e6365e6` |
| Defined, validated, and tested Agent Gateway frame codecs and fixtures | `130c409` through `362bdd3` |
| Migrated the Agent Gateway to the typed frame transport | `9f94b42`, `099ce32`, `e162b2a` |
| Added a provider-oriented project runtime | `043b27c` |
| Added the project console Dashboard experience | `cfb5738` |
| Simplified and tested local and self-hosted workflows | `9771650` through `21d46b2` |
| Added the headless game runtime and authoring foundation | `8df1c26` |
| Completed Short Drama compilation, localization, effects, choices, and runtime validation | `960aed8` |
| Refined the creator-facing Short Drama editor, timeline, Preview, and video playback | `136f7c0` |
| Added the editable Last Train to the Moon sample, media, gameplay capture, and dependency-free web host | `a40f455` |

## How Codex And GPT-5.6 Were Used

The project was organized as a sequence of focused research, specification,
planning, implementation, and live-product sessions. GPT-5.6 was used through
Codex throughout that process. It was not a one-shot prompt and it did not start
with code.

### 1. Start With The Target Creator

Each major phase began in a focused Codex session. I first defined the target
creator and the problem to investigate. GPT-5.6 then helped research
that persona's existing workflow, recurring friction, and expectations for an
AI-native game tool.

The research expanded into reference projects: open-source infrastructure,
commercial creator tools, agent runtimes, OpenClaw's Gateway design, AI-native
games, and relevant papers. Installed research skills, search, and MCP-backed
tools were used to gather and compare evidence. I judged which sources were
relevant and rejected conclusions that did not fit Vifu.

### 2. Turn Research Into A Reviewed Spec

GPT-5.6 organized the accepted findings into a written Markdown specification.
I reviewed and corrected that spec before implementation. This
created a stable statement of the user, the desired outcome, the boundaries of
the product, and the features that should not be built.

After the spec was accepted, Codex Plan Mode converted it into an implementation
plan. I reviewed the plan separately. My important constraint was to define the
final user-visible result first, then work backward into small,
verifiable stages.

For the first Short Drama game, for example, the story script was prepared
before the game was built. Vifu did not hardcode that game. The reusable Short
Drama editor, Canvas, and runtime behavior were implemented first so the game
could be created as a real user would create it.

### 3. Build The Tool, Then Build With The Tool

Codex implemented the accepted plan across the repository, but every feature had
to survive a real creator journey. Agent Browser was used to operate the actual
Dashboard while the editor was still being developed. The game was built inside
Vifu, not inserted directly into the database or assembled with a private script.

This produced a continuous loop:

1. Improve the reusable editor or runtime capability.
2. Use the capability to build the real game.
3. Observe the workflow in the browser.
4. Fix confusing interactions, missing states, or runtime failures immediately.
5. Resume the same creator journey and verify the result.

I also watched the live browser session and provided feedback while Codex was
working, rather than waiting for a finished implementation. Codex could
adjust the current task without losing the accepted outcome or the surrounding
repository constraints.

### 4. Use The Same Workflow For The Demo

The final demonstration follows the same rule. Codex first improved Short
Drama's timeline and Preview behavior. I then assembled narration, soundtrack,
product captures, and two real gameplay segments in a separate Vifu Short Drama
project. The final 2:45 video demonstrates both the product and the process used
to build it.

### Why GPT-5.6 Was Useful

The work required more than isolated code completion. GPT-5.6 had to connect
persona research, reference analysis, product requirements, a reviewed plan,
implementation across a large multi-language repository, and evidence from live
browser and Docker tests. It also had to preserve accepted architectural and
open-source constraints while revising the implementation after new product
feedback.

I retained responsibility for product direction, source quality, architecture,
scope, acceptance criteria, and final review. Codex and
GPT-5.6 made the research-to-working-product loop faster and easier to repeat;
they did not approve their own output.

### Codex Session Evidence

| Session ID | Model | Work covered | Related commits |
| --- | --- | --- | --- |
| `019f4b00-f1fc-72b2-8028-bdd3e2875381` | GPT-5.6 | Runtime architecture, creator workflow, Short Drama implementation, live browser iteration, example game, and submission evidence | `8df1c26`, `960aed8`, `136f7c0`, `a40f455` |

Full prompts and private transcripts are not committed to this public
repository. The session ID and concrete implementation evidence are included
for judging.

## My Decisions

I retained responsibility throughout the workflow:

- deciding what needed to be researched before a feature was designed;
- defining the target persona and turning research into a product direction;
- evaluating the quality, relevance, licensing, and limits of open-source
  references instead of treating generated summaries as facts;
- translating the persona and first-principles analysis into a PRD;
- prioritizing requirements and selecting the smallest useful iteration;
- comparing technical options and making the final language, framework,
  library, provider, protocol, and compatibility choices;
- choosing Rust to keep the core portable across device classes, while
  recognizing that each target still requires its own integration and
  verification work;
- deciding which ideas from OpenClaw, other agent systems, games, and papers
  should be adapted and which did not fit Vifu;
- shaping the Dashboard from strong product references and prior design
  experience rather than copying another product's feature list;
- keeping implementation and product decisions aligned with Vifu's
  open-source direction;
- reviewing the output and test evidence after every iteration, changing the
  plan when the result exposed a better direction, and making the final merge
  and release decisions.

## Verification Record

The final source review produced these results:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo test --workspace --all-targets` | Passed, 137 tests |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `bun run check` | Passed |
| `bun run build` | Passed |
| Focused Dashboard runtime and authoring tests | Passed, 15 tests |
| Example host JavaScript and exported JSON validation | Passed |
| Existing Docker stack | PostgreSQL, Runtime, Agent Gateway, and Dashboard healthy |
| Live creator journey | Passed in the real Dashboard with Agent Browser |
| Gameplay capture | Passed, 74.216 seconds, 1920x1080 H.264/AAC |
| Submission video | Passed, 165 seconds, 1920x1080 H.264/AAC, no subtitle stream |

## How To Test

The repository quickstart is documented in [README.md](README.md). Source build
and verification commands are documented in [BUILD.md](BUILD.md), and operator
guidance is in [docs/self-hosting.md](docs/self-hosting.md). The complete sample
source and media are in
[`examples/last-train-to-the-moon`](examples/last-train-to-the-moon), including
a no-rebuild gameplay capture. A dependency-free endpoint client is in
[`docs/examples/web-short-drama-host`](docs/examples/web-short-drama-host).

## Known Limitations And Open Checks

- The public YouTube URL is stored in the Devpost submission rather than the
  repository.
- Vifu exports editable game source today; direct rendered-video export from the
  Dashboard remains future work.
- Integration tests must use an explicitly isolated test database rather than
  silently falling back to a developer's local PostgreSQL instance.
- Database migrations added during the event require a final compatibility and
  upgrade review.
