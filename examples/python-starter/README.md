# Python Starter

This example creates a real App in the personal Vifu Server and runs a Python
provider inside its native Runtime. Its project binding is stored in
`.vifu/app.json` beside the example, so later starts reopen the same App.

Create an environment from the repository root. Then install and run the
example:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
python examples/python-starter/main.py
```

The program registers the Agent and stays online for application calls. Open
the printed Dashboard URL and select **My Apps → python-starter** to inspect its
Device, Agent, and traces. Press
`Ctrl+C` to stop the program. Vifu does not add a chat UI; your application
defines the endpoint input, output, and interaction.

The `--once` option starts or reuses the Server, opens the same App, connects
its Device, performs one in-process invocation, uploads its trace, and closes
the example's resources.

Continue with the complete [Python tutorial](../../docs/get-started/python.md).
