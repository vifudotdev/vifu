# Python Starter

This example runs a Python provider inside the native Vifu Runtime. It
registers one Agent, reports a `decode` stage, invokes its endpoint, and prints
the local trace count.

From the repository root:

```bash
scripts/build-python-sdk.sh
PYTHONPATH=target/python-sdk python3 examples/python-starter/main.py
```

To add Dashboard monitoring, create a one-time pairing code and connect after
registering the Agent:

```python
gateway = runtime.connect(pairing_code, name="Python Starter")
gateway.wait_until_connected()
```

Omit the pairing code on later starts. Continue with the complete
[Python tutorial](../../docs/get-started/python.md).
