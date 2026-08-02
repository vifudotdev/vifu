# ARM optimization TUI

Running `vifu` in an interactive terminal opens the live Runtime screen. It is
designed for watching many game agents at once, finding the first failing
boundary, and comparing the local models already configured on the device.
The Runtime keeps serving requests while you move between views or open the
Dashboard and an external editor.

## Main screen

Each lane represents one agent and capability. A lane shows its resolved
provider and model, elapsed time, current observed stage, and result. The rail
is elapsed time, not a guessed completion percentage. The header reports the
Vifu process CPU and RSS sampled from the operating system and the number of
resident local models.

The default Attention order puts failures, timeouts, and long-running requests
first. The screen renders only its visible window, so selection and scrolling
remain stable when the Runtime is receiving updates for hundreds of lanes.

| Key | Action |
| --- | --- |
| `↑` / `↓` | Select a lane, trace, or comparison |
| `→` / `Enter` | Open recent traces or the selected trace |
| `←` / `Esc` | Return to the previous view |
| `F` | Cycle All, Running, Problems, and Passed |
| `/` | Search agents and models |
| `S` | Cycle Attention, Recent, and Agent order |
| `Tab` | Cycle Summary, I/O, Metadata, Scores, and Events |
| `T` | Switch the trace view between Tree and Timeline |
| `E` | Open a private, redacted trace export in `$VISUAL`, `$EDITOR`, or `vi` |
| `O` | Measure configured local-model candidates |
| `A` | Activate the selected measured route combination for this session |
| `U` | Restore the previous session routes |
| `B` | Open the same project trace in the Dashboard |
| `Q` | Quit; active comparisons, requests, and route overrides require confirmation |

`NO_COLOR` disables color without removing the symbols and status text.

## Trace evidence

The inspector uses the same Trace, Observation, Generation, Event, and Score
concepts as the Dashboard. Provider stages are shown only when the provider
actually emits them:

`Queue → Load → Tokenize → Prefill → First token → Decode → Validate → Deliver`

Application feedback can then add `Output accepted`, `Action applied`, and
`Frame presented`. Missing feedback stays `unknown`; it is never presented as a
failure. A passed Runtime result with no application feedback means exactly
`Runtime passed · Application outcome unknown`.

The included [StarDojo adapter](../integrations/stardojo/README.md) demonstrates
the three application boundaries. It associates feedback with Vifu's canonical
invocation ID, so a delivered response can be distinguished from a parser,
game-action, or next-frame failure.

## Compare configured local models

Optimize starts with the latest successful real request for each active
agent/capability. Vifu runs only replay-safe capabilities and does not send the
benchmark output back to the game. Every configured compatible local provider
is either executed or shown with an exclusion reason such as capability
mismatch, unavailable model, load failure, insufficient memory, or contract
failure.

Configured remote providers remain visible as **remote fallbacks · not
measured**. Optimize keeps the default comparison local-first and does not
count those fallback routes as local coverage or failures.

Vifu intentionally does not run an unbounded Cartesian search. It measures at
most eight explainable combinations:

1. the current routes;
2. the fastest passing local candidate for each route;
3. the lowest observed Vifu process RSS delta among passing candidates;
4. a combination that reuses a passing shared model where possible;
5. up to four second-choice substitutions for the slowest routes.

Configured remote Providers remain visible in a **Remote fallbacks** inventory,
but Vifu does not count them as tested, passed, or failed local candidates. The
default comparison answers which configured local combination is best on this
device; a remote fallback stays available for the normal Runtime route.

Each final combination has one first run and three repeats. The TUI shows the
repeat median, range, sample count, request TTFT when available, output rate,
and peak Vifu process RSS. It also reports whether model-load telemetry observed
a real load or a resident model. The workload is labelled **sequential replay**
and **runtime/contract verified**: application feedback comes only from the
next real game request after activation.

Activation is an atomic, process-local route change at the invocation boundary.
In-flight requests keep their existing provider, new requests use the selected
combination, and Undo restores the previous routes. It does not modify the
published Deployment.

## Resource behavior

Configured llama.cpp providers are registered cheaply and load on their first
request. Providers with the same effective model configuration share one model
instance. Under memory pressure, idle models are evicted before a new model is
admitted; active calls retain their instance. The admission budget is a
conservative internal estimate. User-facing memory evidence remains the sampled
Vifu process RSS.

| Value | Source and scope |
| --- | --- |
| CPU | Operating-system process CPU time for Vifu; it may exceed 100% across cores |
| RSS / peak RSS | Operating-system memory for the complete Vifu process, not one agent or model |
| Physical memory | Operating-system device total |
| Stage duration | Provider monotonic timing for the named stage |
| TTFT | Request start through first token, only when request-level telemetry is available |
| Input/output tokens and output rate | Provider usage and Decode telemetry |
| Application outcome | Typed feedback from the application boundary |
| Arm Performance Studio | External capture aligned by comparison/trace time window; not synthesized by Vifu |

The TUI and Dashboard never label operating-system samples as Arm Performance
Studio metrics. Use the Dashboard for persistent Trace and Comparison history,
and an Arm tool when hardware-counter or system-wide evidence is required.
