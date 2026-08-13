"""Google ADK tool adapter for an embedded Vifu Runtime."""

from __future__ import annotations

from typing import Any

from vifu import VifuRuntime


def create_vifu_tool(runtime: VifuRuntime, endpoint: str):
    """Create a plain Python function that Google ADK can use as a tool."""

    def ask_on_device(prompt: str) -> dict[str, Any]:
        """Run a prompt through the on-device Vifu agent.

        Args:
            prompt: The task that the on-device agent must complete.

        Returns:
            The agent output and the Vifu invocation ID.
        """

        invocation = runtime.invoke(endpoint, {"prompt": prompt}, session_id="google-adk")
        return {
            "output": invocation.output,
            "vifu_invocation_id": invocation.invocation_id,
        }

    return ask_on_device
