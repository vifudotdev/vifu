# Python Starter

This example runs a Python provider inside the native Vifu Runtime. The SDK
starts or reuses the local Server. Then it joins the permanent Local app.

Create an environment from the repository root. Then install and run the
example:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
python examples/python-starter/main.py
```

The program registers the Agent and opens a terminal prompt. It also stays
online for remote calls. Open the printed Dashboard URL to inspect the Python
Gateway and traces. Enter `/quit` or press `Ctrl+C` to stop the program.

The `--once` option runs only the in-process invocation.

Continue with the complete [Python tutorial](../../docs/get-started/python.md).
