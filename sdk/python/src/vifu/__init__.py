"""Python SDK for the embedded Vifu Runtime, Gateway, and local Server."""

from pathlib import Path


def _use_generated_source_checkout_runtime() -> None:
    """Find native build output when this package is installed editable."""
    package_dir = Path(__file__).resolve().parent
    if (package_dir / "vifu_mobile_ffi.py").is_file():
        return
    for parent in package_dir.parents:
        if not (parent / "Cargo.toml").is_file():
            continue
        generated = parent / "target" / "python-sdk" / "vifu"
        if (generated / "vifu_mobile_ffi.py").is_file():
            __path__.append(str(generated))
            return


_use_generated_source_checkout_runtime()

from ._version import __version__
from ._runtime_version import __runtime_version__
from .app import Vifu
from .gateway import GatewayPairing, VifuGateway
from .providers import LocalLlama, LocalProviderError, LocalWhisper
from .runtime import (
    AgentRequest,
    AgentResponse,
    AgentTrace,
    Invocation,
    VifuRuntime,
)
from .server import VifuServer, VifuServerConfig

__all__ = [
    "AgentRequest",
    "AgentResponse",
    "AgentTrace",
    "GatewayPairing",
    "Invocation",
    "LocalLlama",
    "LocalProviderError",
    "LocalWhisper",
    "Vifu",
    "VifuGateway",
    "VifuRuntime",
    "VifuServer",
    "VifuServerConfig",
    "__version__",
    "__runtime_version__",
]
