# Python Starter

This example runs a Python provider inside the native Vifu Runtime. It connects
to the local Vifu Server and joins its permanent Local app. It does not use a
pairing code.

Start the Vifu release binary in one terminal:

```bash
./vifu
```

Create an environment from the repository root in another terminal. Then
install and run the example:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install vifu
python examples/python-starter/main.py
```

The program registers the Agent, invokes it once, uploads its trace, and stays
online for remote calls. Press `B` in Vifu to inspect the Python Gateway and
trace. Stop the program with `Ctrl+C`.

The `--once` option runs only the in-process invocation. The release workflow
uses this option to test each wheel.

Continue with the complete [Python tutorial](../../docs/get-started/python.md).
