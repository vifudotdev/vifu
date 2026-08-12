# Integrations

`integrations/` contains reusable adapters between Vifu and an external engine,
host, or application boundary. An integration is not a standalone demo and
must not own product-specific assets, model weights, or generated builds.

| Integration | Contract | Runnable example |
| --- | --- | --- |
| [`godot/apple`](godot/apple/) | Swift bridge between an embedded Vifu Runtime and a host-owned libgodot instance | [`examples/godot-ios-starter`](../examples/godot-ios-starter/) |

Runnable applications belong in [`examples/`](../examples/README.md). Shared
integration code must stay independent of a particular character, game asset,
or demo configuration.
