# Build and Manage Agent Apps

An App is the developer-facing unit in Vifu. A personal Vifu Server can hold
many Apps, and each App can contain many Agents. This makes the Dashboard a
personal Agent App workspace instead of one global runtime configuration.

```text
Personal Vifu Server
├── Web Research
│   ├── web-search
│   ├── researcher
│   └── publisher
└── Product Support
    ├── router
    ├── documentation
    └── responder
```

Each App keeps its own Providers, Devices, sessions, endpoints, settings, and
traces. A Device can be a Python process, mobile application, game, computer,
or embedded system running the Vifu Gateway.

## Create an App from Code

Python projects use the shortest App-first path:

```python
from vifu import Vifu

app = Vifu("company-news")


@app.agent("researcher", capability="research")
def research(request):
    return search_and_summarize(request.input["company"])


app.run(run_my_app)
```

The first run creates `company-news` in the personal Server. Vifu writes its
stable Server-issued App ID to `.vifu/app.json` in the project. Later runs from
that project reopen the same App. The file is small, readable local application
metadata; it is not a credential. Vifu ignores this file in Git by default so
each developer can bind the source checkout to their own Server App.

## Register Many Agents

Call `app.agent(...)` once for each distinct capability. Agent IDs remain
explicit in source code and visible in the Dashboard. Vifu does not generate
hidden agents or replace application logic.

```python
app.agent("researcher", research)
app.agent("fact-checker", check_facts)
app.agent("publisher", publish)
```

Application code decides when these Agents run and how data moves between
them. Vifu records their identity, session, stages, latency, result status, and
selected content according to the App's trace policy.

## Use My Apps

Open the Dashboard and select **My Apps**. From there:

- **Overview** shows App health and current trace measurements.
- **Devices** connects phones, computers, games, and embedded runtimes.
- **Agents** manages the capabilities exposed by those runtimes.
- **Providers** connects model and framework implementations.
- **API** creates stable application endpoints and scoped keys.
- **Traces** explains each invocation and its measured stages.
- **Settings** contains App identity and advanced multi-environment controls.

The current open-source workspace manages Apps on one Vifu Server. Portable App
packaging and optional catalog distribution are separate lifecycle features;
the local App and its source code remain under the developer's control.
